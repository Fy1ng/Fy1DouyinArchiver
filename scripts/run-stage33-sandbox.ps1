param(
  [string]$Installer = "src-tauri/target/release/bundle/nsis/Fy1DouyinArchiver_0.1.0_x64-setup.exe",
  [Parameter(Mandatory = $true)][string]$ExpectedSha256,
  [ValidateRange(1, 120)][int]$IdleMinutes = 10,
  [ValidateRange(1, 180)][int]$TimeoutMinutes = 30,
  [string]$OutputRoot = "data/verification/stage33-sandbox",
  [switch]$ProtectedClient,
  [switch]$ForceInstallWebView2,
  [switch]$UseHostWebView2Runtime
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$workspace = (Resolve-Path -LiteralPath ".").Path
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$actualHash = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualHash -ne $ExpectedSha256.ToLowerInvariant()) { throw "installer SHA-256 mismatch" }

$sandbox = (Get-Command WindowsSandbox.exe -ErrorAction Stop).Source
$feature = (dism.exe /Online /English /Get-FeatureInfo /FeatureName:Containers-DisposableClientVM 2>&1 | Select-String "State : Enabled")
if ($null -eq $feature) { throw "Windows Sandbox feature is not enabled" }

$runId = [guid]::NewGuid().ToString("N")
$runRoot = [IO.Path]::GetFullPath((Join-Path $workspace "$OutputRoot/$runId"))
$inputDir = Join-Path $runRoot "input"
$outputDir = Join-Path $runRoot "output"
New-Item -ItemType Directory -Path $inputDir,$outputDir -Force | Out-Null
Copy-Item -LiteralPath $installerPath -Destination (Join-Path $inputDir (Split-Path -Leaf $installerPath))
Copy-Item -LiteralPath (Join-Path $workspace "scripts/sandbox-stage33-guest.ps1") -Destination (Join-Path $inputDir "sandbox-stage33-guest.ps1")

if ($ForceInstallWebView2 -and $UseHostWebView2Runtime) {
  throw "ForceInstallWebView2 and UseHostWebView2Runtime are mutually exclusive"
}

$webView2BootstrapperHash = $null
if ($ForceInstallWebView2) {
  $webView2BootstrapperPath = Join-Path $inputDir "MicrosoftEdgeWebview2Setup.exe"
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  Invoke-WebRequest -UseBasicParsing -Uri "https://go.microsoft.com/fwlink/p/?LinkId=2124703" -OutFile $webView2BootstrapperPath
  if (-not (Test-Path -LiteralPath $webView2BootstrapperPath -PathType Leaf)) {
    throw "WebView2 bootstrapper download did not produce a file"
  }
  $signature = Get-AuthenticodeSignature -LiteralPath $webView2BootstrapperPath
  if ($signature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
      $null -eq $signature.SignerCertificate -or
      $signature.SignerCertificate.Subject -notmatch "Microsoft Corporation") {
    throw "WebView2 bootstrapper Authenticode signature is not a valid Microsoft signature"
  }
  $webView2BootstrapperHash = (Get-FileHash -LiteralPath $webView2BootstrapperPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

$hostWebView2RuntimePath = $null
if ($UseHostWebView2Runtime) {
  $hostWebView2Root = Join-Path ${env:ProgramFiles(x86)} "Microsoft\EdgeWebView\Application"
  $hostWebView2Runtime = Get-ChildItem -LiteralPath $hostWebView2Root -Directory -Force -ErrorAction Stop |
    Where-Object { $_.Name -match '^\d+(\.\d+){3}$' } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
  if ($null -eq $hostWebView2Runtime) { throw "host WebView2 Runtime version directory is missing" }
  $hostWebView2RuntimePath = $hostWebView2Runtime.FullName
  foreach ($relativePath in @("msedgewebview2.exe", "msedge.dll", "icudtl.dat")) {
    $runtimeFile = Join-Path $hostWebView2RuntimePath $relativePath
    if (-not (Test-Path -LiteralPath $runtimeFile -PathType Leaf)) {
      throw "host WebView2 Runtime is incomplete: $relativePath"
    }
  }
  $runtimeSignature = Get-AuthenticodeSignature -LiteralPath (Join-Path $hostWebView2RuntimePath "msedgewebview2.exe")
  if ($runtimeSignature.Status -ne [Management.Automation.SignatureStatus]::Valid -or
      $null -eq $runtimeSignature.SignerCertificate -or
      $runtimeSignature.SignerCertificate.Subject -notmatch "Microsoft Corporation") {
    throw "host WebView2 Runtime does not have a valid Microsoft signature"
  }
}

$wsbPath = Join-Path $runRoot "stage33.wsb"
$webView2Argument = if ($ForceInstallWebView2) {
  ' -ForceInstallWebView2 -ExpectedWebView2BootstrapperSha256 "' + $webView2BootstrapperHash + '"'
} elseif ($UseHostWebView2Runtime) {
  " -UseMappedWebView2Runtime"
} else {
  ""
}
$logonCommand = 'powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "C:\EmilyInput\sandbox-stage33-guest.ps1" -InputDir "C:\EmilyInput" -OutputDir "C:\EmilyOutput" -ExpectedSha256 "' + $actualHash + '" -IdleMinutes ' + $IdleMinutes + $webView2Argument
$escapedInput = [Security.SecurityElement]::Escape($inputDir)
$escapedOutput = [Security.SecurityElement]::Escape($outputDir)
$escapedCommand = [Security.SecurityElement]::Escape($logonCommand)
$protectedClientXml = if ($ProtectedClient) { "  <ProtectedClient>Enable</ProtectedClient>" } else { "" }
$hostWebView2RuntimeXml = if ($UseHostWebView2Runtime) {
  $escapedHostWebView2RuntimePath = [Security.SecurityElement]::Escape($hostWebView2RuntimePath)
@"
    <MappedFolder>
      <HostFolder>$escapedHostWebView2RuntimePath</HostFolder>
      <SandboxFolder>C:\EmilyWebView2Runtime</SandboxFolder>
      <ReadOnly>true</ReadOnly>
    </MappedFolder>
"@
} else {
  ""
}
$wsb = @"
<Configuration>
  <MappedFolders>
    <MappedFolder>
      <HostFolder>$escapedInput</HostFolder>
      <SandboxFolder>C:\EmilyInput</SandboxFolder>
      <ReadOnly>true</ReadOnly>
    </MappedFolder>
    <MappedFolder>
      <HostFolder>$escapedOutput</HostFolder>
      <SandboxFolder>C:\EmilyOutput</SandboxFolder>
      <ReadOnly>false</ReadOnly>
    </MappedFolder>
$hostWebView2RuntimeXml
  </MappedFolders>
  <LogonCommand>
    <Command>$escapedCommand</Command>
  </LogonCommand>
  <ClipboardRedirection>Disable</ClipboardRedirection>
  <PrinterRedirection>Disable</PrinterRedirection>
  <VideoInput>Disable</VideoInput>
  <AudioInput>Disable</AudioInput>
$protectedClientXml
</Configuration>
"@
[IO.File]::WriteAllText($wsbPath, $wsb, [Text.UTF8Encoding]::new($false))

$sandboxProcess = Start-Process -FilePath $sandbox -ArgumentList "`"$wsbPath`"" -PassThru
$resultPath = Join-Path $outputDir "stage33-sandbox-result.json"
$progressPath = Join-Path $outputDir "stage33-sandbox-progress.json"
$deadline = [DateTime]::UtcNow.AddMinutes($TimeoutMinutes)
$lastStage = $null
do {
  if (Test-Path -LiteralPath $resultPath -PathType Leaf) { break }
  if ($sandboxProcess.HasExited) { throw "Windows Sandbox exited before producing a result" }
  if (Test-Path -LiteralPath $progressPath -PathType Leaf) {
    try {
      $stage = (Get-Content -LiteralPath $progressPath -Raw | ConvertFrom-Json).stage
      if ($stage -and $stage -ne $lastStage) {
        Write-Host "Sandbox stage: $stage"
        $lastStage = $stage
      }
    } catch {}
  }
  Start-Sleep -Seconds 1
} while ([DateTime]::UtcNow -lt $deadline)

if (-not (Test-Path -LiteralPath $resultPath -PathType Leaf)) {
  if (-not $sandboxProcess.HasExited) { $sandboxProcess.CloseMainWindow() | Out-Null }
  throw "Windows Sandbox verification timed out at stage: $lastStage"
}

$result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
if (-not $sandboxProcess.HasExited) {
  $sandboxProcess.CloseMainWindow() | Out-Null
  $sandboxProcess.WaitForExit(15000) | Out-Null
}
if ($result.status -ne "passed") {
  throw "Windows Sandbox verification failed: $($result.error)"
}

Write-Host "Windows Sandbox Stage 3.3 verification passed."
Write-Host "Result: $resultPath"
Write-Host "SHA-256: $actualHash"
