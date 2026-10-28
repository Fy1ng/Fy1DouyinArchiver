param(
  [Parameter(Mandatory = $true)][string]$InputDir,
  [Parameter(Mandatory = $true)][string]$OutputDir,
  [Parameter(Mandatory = $true)][string]$ExpectedSha256,
  [ValidateRange(1, 120)][int]$IdleMinutes = 10,
  [switch]$ForceInstallWebView2,
  [string]$ExpectedWebView2BootstrapperSha256 = "",
  [switch]$UseMappedWebView2Runtime
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public static class EmilyWindowText {
  public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] static extern bool EnumChildWindows(IntPtr hWndParent, EnumWindowsProc lpEnumFunc, IntPtr lParam);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);
  public static string[] ChildTexts(IntPtr parent) {
    var values = new List<string>();
    EnumChildWindows(parent, (handle, state) => {
      var buffer = new StringBuilder(4096);
      GetWindowText(handle, buffer, buffer.Capacity);
      var value = buffer.ToString().Trim();
      if (value.Length > 0) values.Add(value);
      return true;
    }, IntPtr.Zero);
    return values.ToArray();
  }
}
"@

$resultPath = Join-Path $OutputDir "stage33-sandbox-result.json"
$diagnosticPath = Join-Path $OutputDir "stage33-sandbox-diagnostic.txt"
$progressPath = Join-Path $OutputDir "stage33-sandbox-progress.json"
$events = [Collections.Generic.List[object]]::new()
$samples = [Collections.Generic.List[object]]::new()
$startedAt = [DateTime]::UtcNow
$installDir = Join-Path $env:LOCALAPPDATA "EmilyStage33Audit"
$bootstrapDir = Join-Path $env:APPDATA "lifearchive.douyin-history-archive"
$dataRoot = Join-Path $bootstrapDir "Data"
$installer = Get-ChildItem -LiteralPath $InputDir -Filter "*-setup.exe" -File | Select-Object -First 1
$appProcess = $null

function Add-Event([string]$Name, [bool]$Passed, [hashtable]$Evidence = @{}) {
  $events.Add([ordered]@{
    name = $Name
    passed = $Passed
    at = [DateTime]::UtcNow.ToString("o")
    evidence = $Evidence
  })
}

function Write-Utf8NoBom([string]$Path, [string]$Contents) {
  [IO.File]::WriteAllText($Path, $Contents, [Text.UTF8Encoding]::new($false))
}

function Write-Progress([string]$Stage) {
  $temporary = "$progressPath.tmp"
  $contents = [ordered]@{ stage = $Stage; at = [DateTime]::UtcNow.ToString("o") } |
    ConvertTo-Json -Compress
  Write-Utf8NoBom $temporary $contents
  Move-Item -LiteralPath $temporary -Destination $progressPath -Force
}

function Require([bool]$Condition, [string]$Message) {
  if (-not $Condition) { throw $Message }
}

function Wait-Until([scriptblock]$Condition, [int]$TimeoutSeconds, [string]$Message) {
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  do {
    if (& $Condition) { return }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  throw $Message
}

function Get-DescendantProcesses([int]$RootPid) {
  $all = @(Get-CimInstance Win32_Process -ErrorAction Stop)
  $selected = [Collections.Generic.Dictionary[int,object]]::new()
  $pending = [Collections.Generic.Queue[int]]::new()
  $pending.Enqueue($RootPid)
  while ($pending.Count -gt 0) {
    $parent = $pending.Dequeue()
    foreach ($item in $all) {
      if ([int]$item.ProcessId -eq $RootPid -or [int]$item.ParentProcessId -ne $parent) { continue }
      $childProcessId = [int]$item.ProcessId
      if ($selected.ContainsKey($childProcessId)) { continue }
      $selected.Add($childProcessId, $item)
      $pending.Enqueue($childProcessId)
    }
  }
  $root = $all | Where-Object { [int]$_.ProcessId -eq $RootPid } | Select-Object -First 1
  if ($null -ne $root) { $selected[$RootPid] = $root }
  @($selected.Values)
}

function Get-ProcessCpu([int]$ProcessId) {
  $process = Get-Process -Id $ProcessId -ErrorAction Stop
  [double]$process.TotalProcessorTime.TotalSeconds
}

function Get-ProcessIdentity([object]$Process) {
  [ordered]@{
    processId = [int]$Process.ProcessId
    creationDate = [string]$Process.CreationDate
    name = [string]$Process.Name
    executablePath = [string]$Process.ExecutablePath
  }
}

function Test-ProcessIdentityAlive([object]$Identity) {
  $processId = [int]$Identity.processId
  $current = Get-CimInstance Win32_Process -Filter "ProcessId=$processId" -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if ($null -eq $current) { return $false }
  [string]$current.CreationDate -eq [string]$Identity.creationDate
}

function Percentile([double[]]$Values, [double]$Probability) {
  if ($Values.Count -eq 0) { return $null }
  $sorted = @($Values | Sort-Object)
  $index = [Math]::Ceiling($Probability * $sorted.Count) - 1
  $index = [Math]::Max(0, [Math]::Min($sorted.Count - 1, $index))
  [double]$sorted[$index]
}

function Close-MainWindow([Diagnostics.Process]$Process) {
  $Process.Refresh()
  $closed = $Process.CloseMainWindow()
  if (-not $closed) { throw "main window did not accept WM_CLOSE" }
  if (-not $Process.WaitForExit(15000)) { throw "controlled exit exceeded 15 seconds" }
}

function Get-AclSummary([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return @() }
  try {
    @((Get-Acl -LiteralPath $Path -ErrorAction Stop).Access | ForEach-Object {
      [ordered]@{
        identity = [string]$_.IdentityReference
        rights = [string]$_.FileSystemRights
        type = [string]$_.AccessControlType
        inherited = [bool]$_.IsInherited
      }
    })
  } catch {
    @([ordered]@{ error = $_.Exception.Message })
  }
}

function Test-ReadableFile([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
  try {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite -bor [IO.FileShare]::Delete)
    $stream.Dispose()
    $true
  } catch {
    $false
  }
}

function Get-WebView2State {
  $registry = [Collections.Generic.List[object]]::new()
  foreach ($path in @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
  )) {
    if (-not (Test-Path -LiteralPath $path)) { continue }
    $value = Get-ItemProperty -LiteralPath $path -ErrorAction SilentlyContinue
    $registry.Add([ordered]@{
      path = $path
      pv = $value.pv
      name = $value.name
      location = $value.location
    })
  }

  $roots = [Collections.Generic.List[object]]::new()
  foreach ($root in @(
    "$env:ProgramFiles\Microsoft\EdgeWebView\Application",
    "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application",
    "$env:LOCALAPPDATA\Microsoft\EdgeWebView\Application"
  ) | Where-Object { $_ }) {
    $versions = [Collections.Generic.List[object]]::new()
    if (Test-Path -LiteralPath $root -PathType Container) {
      Get-ChildItem -LiteralPath $root -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -match '^\d+(\.\d+){3}$' } |
        Sort-Object { try { [version]$_.Name } catch { [version]'0.0.0.0' } } -Descending |
        Select-Object -First 5 |
        ForEach-Object {
          $runtimeExe = Join-Path $_.FullName "msedgewebview2.exe"
          $edgeDll = Join-Path $_.FullName "msedge.dll"
          $icuData = Join-Path $_.FullName "icudtl.dat"
          $versions.Add([ordered]@{
            version = $_.Name
            path = $_.FullName
            runtimeExeExists = (Test-Path -LiteralPath $runtimeExe -PathType Leaf)
            runtimeExeReadable = (Test-ReadableFile $runtimeExe)
            runtimeExeBytes = if (Test-Path -LiteralPath $runtimeExe -PathType Leaf) { (Get-Item -LiteralPath $runtimeExe).Length } else { $null }
            edgeDllExists = (Test-Path -LiteralPath $edgeDll -PathType Leaf)
            icuDataExists = (Test-Path -LiteralPath $icuData -PathType Leaf)
            acl = @(Get-AclSummary $_.FullName)
            runtimeExeAcl = @(Get-AclSummary $runtimeExe)
          })
        }
    }
    $roots.Add([ordered]@{
      path = $root
      exists = (Test-Path -LiteralPath $root -PathType Container)
      acl = @(Get-AclSummary $root)
      versions = @($versions)
    })
  }

  $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
  $principal = [Security.Principal.WindowsPrincipal]::new($identity)
  [ordered]@{
    identity = $identity.Name
    elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    registry = @($registry)
    roots = @($roots)
  }
}

function Get-FailureSnapshot {
  $roots = @($env:APPDATA, $env:LOCALAPPDATA) | Where-Object { $_ -and (Test-Path -LiteralPath $_) }
  $files = [Collections.Generic.List[object]]::new()
  foreach ($root in $roots) {
    Get-ChildItem -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue |
      Where-Object {
        $_.FullName -match "(?i)(lifearchive|douyin-history|LifeArchiveProject[\\/]Emily|EmilyStage33Audit)"
      } |
      Select-Object -First 300 |
      ForEach-Object {
        $files.Add([ordered]@{
          path = $_.FullName
          isDirectory = $_.PSIsContainer
          bytes = if ($_.PSIsContainer) { $null } else { $_.Length }
        })
      }
  }
  $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue |
    Where-Object {
      $_.Name -match "(?i)(douyin-history-archive|msedgewebview2|msedge|chrome)" -or
      ($_.CommandLine -and $_.CommandLine -match "(?i)(EmilyStage33Audit|douyin-history-archive|Fy1DouyinArchiver)")
    } |
    Select-Object Name,ProcessId,ParentProcessId,ExecutablePath,CommandLine)
  $windows = @(Get-Process -ErrorAction SilentlyContinue |
    Where-Object { $_.MainWindowHandle -ne 0 } |
    ForEach-Object {
      [ordered]@{
        processName = $_.ProcessName
        id = $_.Id
        mainWindowTitle = $_.MainWindowTitle
        mainWindowHandle = [long]$_.MainWindowHandle
        childTexts = @([EmilyWindowText]::ChildTexts($_.MainWindowHandle))
      }
    })
  $webViewRegistry = [Collections.Generic.List[object]]::new()
  foreach ($path in @(
    "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKLM:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}",
    "HKCU:\SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}"
  )) {
    if (Test-Path -LiteralPath $path) {
      $value = Get-ItemProperty -LiteralPath $path -ErrorAction SilentlyContinue
      $webViewRegistry.Add([ordered]@{
        path = $path
        pv = $value.pv
        name = $value.name
        location = $value.location
      })
    }
  }
  $webViewPaths = @(
    "$env:ProgramFiles\Microsoft\EdgeWebView\Application",
    "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application",
    "$env:LOCALAPPDATA\Microsoft\EdgeWebView\Application",
    "$env:LOCALAPPDATA\Microsoft\Edge\Application"
  ) | ForEach-Object {
    [ordered]@{ path = $_; exists = (Test-Path -LiteralPath $_) }
  }
  [ordered]@{
    appData = $env:APPDATA
    localAppData = $env:LOCALAPPDATA
    files = @($files)
    processes = $processes
    windows = $windows
    webViewRegistry = @($webViewRegistry)
    webViewPaths = @($webViewPaths)
  }
}

New-Item -ItemType Directory -Path $OutputDir -Force | Out-Null
Write-Progress "started"

try {
  Require ($null -ne $installer) "installer missing from mapped input"
  $actualHash = (Get-FileHash -LiteralPath $installer.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  Require ($actualHash -eq $ExpectedSha256.ToLowerInvariant()) "installer SHA-256 mismatch"
  Add-Event "installer_hash" $true @{ sha256 = $actualHash; bytes = $installer.Length }

  Write-Progress "capturing_webview2_before_install"
  $webView2BeforeInstall = Get-WebView2State
  Add-Event "webview2_before_install" $true @{ state = $webView2BeforeInstall }

  Write-Progress "installing"
  Require (-not (Test-Path -LiteralPath $installDir)) "sandbox install directory is not clean"
  Require (-not (Test-Path -LiteralPath $bootstrapDir)) "sandbox bootstrap directory is not clean"
  $install = Start-Process -FilePath $installer.FullName -ArgumentList @("/S", "/D=$installDir") -Wait -PassThru -WindowStyle Hidden
  Require ($install.ExitCode -eq 0) "silent clean install failed: $($install.ExitCode)"
  $appExe = Join-Path $installDir "Fy1DouyinArchiver.exe"
  Require (Test-Path -LiteralPath $appExe -PathType Leaf) "installed app executable missing"
  Require (Test-Path -LiteralPath (Join-Path $installDir "uninstall.exe") -PathType Leaf) "installed uninstaller missing"
  Add-Event "clean_install" $true @{ installDir = $installDir }

  Write-Progress "capturing_webview2_after_installer"
  $webView2AfterInstaller = Get-WebView2State
  Add-Event "webview2_after_installer" $true @{ state = $webView2AfterInstaller }

  if ($ForceInstallWebView2) {
    Write-Progress "installing_webview2_bootstrapper"
    Require ($ExpectedWebView2BootstrapperSha256 -match '^[0-9a-fA-F]{64}$') "expected WebView2 bootstrapper SHA-256 is invalid"
    $bootstrapperPath = Join-Path $InputDir "MicrosoftEdgeWebview2Setup.exe"
    Require (Test-Path -LiteralPath $bootstrapperPath -PathType Leaf) "mapped WebView2 bootstrapper is missing"
    $bootstrapperHash = (Get-FileHash -LiteralPath $bootstrapperPath -Algorithm SHA256).Hash.ToLowerInvariant()
    Require ($bootstrapperHash -eq $ExpectedWebView2BootstrapperSha256.ToLowerInvariant()) "mapped WebView2 bootstrapper SHA-256 mismatch"
    $bootstrapper = Start-Process -FilePath $bootstrapperPath -ArgumentList @("/silent", "/install") -Wait -PassThru -WindowStyle Hidden
    $webView2AfterBootstrapper = Get-WebView2State
    Add-Event "webview2_forced_bootstrapper" ($bootstrapper.ExitCode -eq 0) @{
      exitCode = $bootstrapper.ExitCode
      sha256 = $bootstrapperHash
      bytes = (Get-Item -LiteralPath $bootstrapperPath).Length
      state = $webView2AfterBootstrapper
    }
  }

  if ($UseMappedWebView2Runtime) {
    $mappedRuntime = "C:\EmilyWebView2Runtime"
    foreach ($relativePath in @("msedgewebview2.exe", "msedge.dll", "icudtl.dat")) {
      Require (Test-Path -LiteralPath (Join-Path $mappedRuntime $relativePath) -PathType Leaf) "mapped WebView2 Runtime is incomplete: $relativePath"
    }
    $env:WEBVIEW2_BROWSER_EXECUTABLE_FOLDER = $mappedRuntime
    Add-Event "mapped_webview2_runtime" $true @{
      path = $mappedRuntime
      runtimeExeBytes = (Get-Item -LiteralPath (Join-Path $mappedRuntime "msedgewebview2.exe")).Length
    }
  }

  Write-Progress "launching"
  $appProcess = Start-Process -FilePath $appExe -PassThru
  Wait-Until { $appProcess.Refresh(); -not $appProcess.HasExited -and $appProcess.MainWindowHandle -ne 0 } 30 "manual launch did not show the main window"
  Wait-Until { Test-Path -LiteralPath (Join-Path $bootstrapDir "device-settings.json") } 30 "device settings were not initialized"
  Wait-Until { Test-Path -LiteralPath (Join-Path $dataRoot "history.sqlite") } 30 "database was not initialized"
  Require (-not (Test-Path -LiteralPath (Join-Path $bootstrapDir "shutdown-intent.json"))) "shutdown marker present after startup"
  Add-Event "manual_launch_visible" $true @{ pid = $appProcess.Id; mainWindowHandle = [long]$appProcess.MainWindowHandle }

  Write-Progress "second_instance"
  $second = Start-Process -FilePath $appExe -PassThru
  Require ($second.WaitForExit(10000)) "second manual instance did not exit"
  Require ($second.ExitCode -eq 0) "second manual instance failed: $($second.ExitCode)"
  Require (-not $appProcess.HasExited) "primary instance exited during second-instance activation"
  $primaryCount = @(Get-Process -Name "douyin-history-archive" -ErrorAction SilentlyContinue).Count
  Require ($primaryCount -eq 1) "second launch left more than one GUI process"
  Add-Event "second_instance_activation" $true @{ primaryProcessCount = $primaryCount }

  Write-Progress "measuring_idle"
  $logicalProcessors = [Math]::Max(1, [Environment]::ProcessorCount)
  $priorCpu = @{}
  $browserObserved = $false
  $sampleCount = $IdleMinutes * 12
  for ($index = 0; $index -lt $sampleCount; $index++) {
    if ($appProcess.HasExited) { throw "application exited during idle measurement" }
    $tree = @(Get-DescendantProcesses $appProcess.Id)
    $privateWorkingSet = 0.0
    $names = [Collections.Generic.List[string]]::new()
    $cpuDelta = 0.0
    foreach ($entry in $tree) {
      $processId = [int]$entry.ProcessId
      $names.Add([string]$entry.Name)
      if ($entry.Name -match "^(msedge|chrome)\.exe$") { $browserObserved = $true }
      try {
        $counter = Get-CimInstance Win32_PerfRawData_PerfProc_Process -Filter "IDProcess=$processId" -ErrorAction Stop | Select-Object -First 1
        if ($null -ne $counter) { $privateWorkingSet += [double]$counter.WorkingSetPrivate }
        $cpu = Get-ProcessCpu $processId
        if ($priorCpu.ContainsKey($processId)) { $cpuDelta += [Math]::Max(0.0, $cpu - [double]$priorCpu[$processId]) }
        $priorCpu[$processId] = $cpu
      } catch {}
    }
    $cpuPercent = if ($index -eq 0) { 0.0 } else { ($cpuDelta / (5.0 * $logicalProcessors)) * 100.0 }
    $samples.Add([ordered]@{
      at = [DateTime]::UtcNow.ToString("o")
      privateWorkingSetBytes = [long]$privateWorkingSet
      normalizedCpuPercent = [Math]::Round($cpuPercent, 6)
      processCount = $tree.Count
      processNames = @($names | Sort-Object -Unique)
    })
    if ($index + 1 -lt $sampleCount) { Start-Sleep -Seconds 5 }
  }

  $workingSets = [double[]]@($samples | ForEach-Object { [double]$_.privateWorkingSetBytes })
  $cpuValues = [double[]]@($samples | Select-Object -Skip 1 | ForEach-Object { [double]$_.normalizedCpuPercent })
  $resourceSummary = [ordered]@{
    durationMinutes = $IdleMinutes
    sampleIntervalSeconds = 5
    sampleCount = $samples.Count
    logicalProcessors = $logicalProcessors
    privateWorkingSetMedianBytes = [long](Percentile $workingSets 0.5)
    privateWorkingSetP95Bytes = [long](Percentile $workingSets 0.95)
    normalizedCpuAveragePercent = if ($cpuValues.Count -eq 0) { 0.0 } else { [Math]::Round((($cpuValues | Measure-Object -Average).Average), 6) }
    normalizedCpuP95Percent = if ($cpuValues.Count -eq 0) { 0.0 } else { [Math]::Round((Percentile $cpuValues 0.95), 6) }
    browserObserved = $browserObserved
  }
  $resourcePassed = $resourceSummary.privateWorkingSetMedianBytes -le 188743680 -and
    $resourceSummary.privateWorkingSetP95Bytes -le 262144000 -and
    $resourceSummary.normalizedCpuAveragePercent -lt 0.5 -and
    $resourceSummary.normalizedCpuP95Percent -lt 2.0 -and
    -not $resourceSummary.browserObserved
  Add-Event "idle_resources" $resourcePassed $resourceSummary
  Require $resourcePassed "idle resource thresholds failed"

  Write-Progress "controlled_exit"
  $ownedTreeBeforeExit = @(Get-DescendantProcesses $appProcess.Id | ForEach-Object { Get-ProcessIdentity $_ })
  Require ($ownedTreeBeforeExit.Count -gt 1) "application process tree did not include WebView2 descendants before exit"
  Close-MainWindow $appProcess
  $appProcess = $null
  Wait-Until {
    @($ownedTreeBeforeExit | Where-Object { Test-ProcessIdentityAlive $_ }).Count -eq 0
  } 15 "controlled exit left a process from the captured application tree"
  Require (-not (Test-Path -LiteralPath (Join-Path $bootstrapDir "shutdown-intent.json"))) "clean exit left shutdown marker"
  $residualAppProcesses = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
    $_.Name -ieq "Fy1DouyinArchiver.exe" -or
    ($_.CommandLine -and $_.CommandLine -match [Regex]::Escape($installDir))
  })
  Require ($residualAppProcesses.Count -eq 0) "controlled exit left app-related processes"
  Add-Event "controlled_exit" $true @{
    capturedProcessCount = $ownedTreeBeforeExit.Count
    capturedProcessNames = @($ownedTreeBeforeExit | ForEach-Object { $_.name } | Sort-Object -Unique)
    residualCapturedProcessCount = 0
    residualAppProcessCount = 0
  }

  Write-Progress "uninstalling"
  $sentinel = Join-Path $dataRoot "sandbox-retain-sentinel.txt"
  [IO.File]::WriteAllText($sentinel, "retain", [Text.UTF8Encoding]::new($false))
  $uninstaller = Join-Path $installDir "uninstall.exe"
  $uninstall = Start-Process -FilePath $uninstaller -ArgumentList "/S" -PassThru -WindowStyle Hidden
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  do {
    if (-not (Test-Path -LiteralPath $installDir)) { break }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  Require (-not (Test-Path -LiteralPath $installDir)) "ordinary silent uninstall did not remove install directory"
  Require (Test-Path -LiteralPath $sentinel -PathType Leaf) "default uninstall did not retain user data"
  Wait-Until {
    @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
      $_.Name -ieq "uninstall.exe" -or $_.Name -ieq "Un.exe"
    }).Count -eq 0
  } 15 "ordinary uninstall left uninstaller processes"
  $uninstallProcesses = @()
  Require ($uninstallProcesses.Count -eq 0) "ordinary uninstall left uninstaller processes"
  Add-Event "ordinary_uninstall_retain_data" $true @{ bootstrapRetained = (Test-Path -LiteralPath $bootstrapDir) }

  Write-Progress "writing_result"
  $result = [ordered]@{
    schemaVersion = 1
    status = "passed"
    startedAt = $startedAt.ToString("o")
    finishedAt = [DateTime]::UtcNow.ToString("o")
    os = (Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber)
    installerSha256 = $actualHash
    events = @($events)
    resourceSummary = $resourceSummary
    samples = @($samples)
  }
  Write-Utf8NoBom $resultPath ($result | ConvertTo-Json -Depth 10)
  Write-Progress "passed"
} catch {
  $message = $_.Exception.Message
  Write-Utf8NoBom $diagnosticPath $message
  $result = [ordered]@{
    schemaVersion = 1
    status = "failed"
    startedAt = $startedAt.ToString("o")
    finishedAt = [DateTime]::UtcNow.ToString("o")
    error = $message
    events = @($events)
    samples = @($samples)
    failureSnapshot = Get-FailureSnapshot
  }
  Write-Utf8NoBom $resultPath ($result | ConvertTo-Json -Depth 10)
  Write-Progress "failed"
  exit 1
} finally {
  if ($null -ne $appProcess -and -not $appProcess.HasExited) {
    & taskkill.exe /PID $appProcess.Id /T /F *> $null
  }
}
