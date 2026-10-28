param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [Parameter(Mandatory = $true)][string]$ExpectedSha256
)

$ErrorActionPreference = "Stop"
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$actualSha = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha -ne $ExpectedSha256.ToLowerInvariant()) {
  throw "installer SHA-256 mismatch"
}

$tempRoot = [IO.Path]::GetFullPath($env:TEMP).TrimEnd([IO.Path]::DirectorySeparatorChar)
$installRoot = Join-Path $tempRoot ("emily-package-audit-" + [guid]::NewGuid().ToString("N"))
$resolvedInstall = [IO.Path]::GetFullPath($installRoot)
$safePrefix = $tempRoot + [IO.Path]::DirectorySeparatorChar + "emily-package-audit-"
if (-not $resolvedInstall.StartsWith($safePrefix, [StringComparison]::OrdinalIgnoreCase)) {
  throw "unsafe package audit path"
}

function Get-AuditUninstallProcesses {
  param([Parameter(Mandatory = $true)][string]$InstallPath)

  $escaped = [Regex]::Escape($InstallPath)
  @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
    $_.CommandLine -and $_.CommandLine -match $escaped -and
    ($_.Name -ieq "uninstall.exe" -or $_.Name -ieq "Un.exe")
  })
}

function Stop-AuditUninstallProcesses {
  param([Parameter(Mandatory = $true)][string]$InstallPath)

  foreach ($process in Get-AuditUninstallProcesses -InstallPath $InstallPath) {
    & taskkill.exe /PID $process.ProcessId /T /F *> $null
  }
}

try {
  $process = Start-Process -FilePath $installerPath -ArgumentList @("/S", "/D=$resolvedInstall") -Wait -PassThru -WindowStyle Hidden
  if ($process.ExitCode -ne 0) { throw "silent install failed: $($process.ExitCode)" }
  if (-not (Test-Path -LiteralPath $resolvedInstall -PathType Container)) {
    throw "installer did not create the requested audit directory"
  }
  $forbidden = @(Get-ChildItem -LiteralPath $resolvedInstall -Recurse -Force | Where-Object {
    $_.Name -match "(?i)(^node(w)?\.exe$|^index\.cjs$|\.map$|chromium|chrome-headless|headless-shell|^msedge\.exe$|^chrome\.exe$)" -or
    $_.FullName -match "(?i)(playwright|chrome-remote-interface|@vercel[\\/]ncc|[\\/](runtime|collector|node_modules|browsers?)[\\/])"
  })
  if ($forbidden.Count -gt 0) {
    throw "forbidden installed files: $($forbidden.Name -join ', ')"
  }
} finally {
  if (Test-Path -LiteralPath $resolvedInstall) {
    $uninstaller = Get-ChildItem -LiteralPath $resolvedInstall -Filter "*unins*.exe" -File -ErrorAction SilentlyContinue | Select-Object -First 1
    $uninstallError = $null
    if ($null -eq $uninstaller) {
      $uninstallError = "audit installation is missing an uninstaller"
    } else {
      # Package-tree cleanup must not inspect or mutate the developer's real
      # app-data state. Full guarded uninstall is a separate lifecycle gate.
      $uninstall = Start-Process -FilePath $uninstaller.FullName -ArgumentList @("/S", "/UPDATE") -PassThru -WindowStyle Hidden
      $deadline = [DateTime]::UtcNow.AddSeconds(45)
      do {
        $related = @(Get-AuditUninstallProcesses -InstallPath $resolvedInstall)
        if (-not (Test-Path -LiteralPath $resolvedInstall) -and $related.Count -eq 0) {
          break
        }
        Start-Sleep -Milliseconds 100
      } while ([DateTime]::UtcNow -lt $deadline)

      $related = @(Get-AuditUninstallProcesses -InstallPath $resolvedInstall)
      if (Test-Path -LiteralPath $resolvedInstall) {
        Stop-AuditUninstallProcesses -InstallPath $resolvedInstall
        $uninstallError = "silent uninstall did not remove the audit installation"
      } elseif ($related.Count -gt 0) {
        Stop-AuditUninstallProcesses -InstallPath $resolvedInstall
        $uninstallError = "silent uninstall left related processes running"
      } elseif ($uninstall.HasExited -and $uninstall.ExitCode -ne 0) {
        $uninstallError = "silent uninstall failed: $($uninstall.ExitCode)"
      }
    }
    if (-not $resolvedInstall.StartsWith($safePrefix, [StringComparison]::OrdinalIgnoreCase)) {
      throw "unsafe package audit cleanup path"
    }
    if (Test-Path -LiteralPath $resolvedInstall) {
      Remove-Item -LiteralPath $resolvedInstall -Recurse -Force
    }
    if ($null -ne $uninstallError) {
      throw $uninstallError
    }
  }
}

Write-Host "Installed package audit passed: $actualSha"
