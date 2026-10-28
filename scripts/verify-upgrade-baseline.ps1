param(
  [Parameter(Mandatory = $true)][string]$BaselineCommit,
  [Parameter(Mandatory = $true)][string]$BaselineInstaller,
  [Parameter(Mandatory = $true)][string]$BaselineSha256,
  [Parameter(Mandatory = $true)][string]$AppDataDir,
  [Parameter(Mandatory = $true)][string]$UpgradeInstaller,
  [Parameter(Mandatory = $true)][string]$UpgradeSha256
)

$ErrorActionPreference = "Stop"
if ($BaselineCommit -notmatch "^[0-9a-f]{7,40}$") { throw "invalid baseline commit" }
if (Get-Process msedge, chrome -ErrorAction SilentlyContinue) { throw "close Edge and Chrome before upgrade verification" }
foreach ($pair in @(@($BaselineInstaller, $BaselineSha256), @($UpgradeInstaller, $UpgradeSha256))) {
  $path = (Resolve-Path -LiteralPath $pair[0]).Path
  $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($hash -ne $pair[1].ToLowerInvariant()) { throw "installer SHA-256 mismatch: $path" }
}

$template = (Resolve-Path -LiteralPath $AppDataDir).Path
$root = [IO.Path]::GetPathRoot($template)
if ($template.TrimEnd("\") -eq $root.TrimEnd("\")) { throw "app-data template cannot be a drive root" }
$runRoot = Join-Path $env:TEMP ("emily-upgrade-baseline-" + [guid]::NewGuid().ToString("N"))
$safePrefix = [IO.Path]::GetFullPath($env:TEMP).TrimEnd("\") + "\emily-upgrade-baseline-"
$resolvedRun = [IO.Path]::GetFullPath($runRoot)
if (-not $resolvedRun.StartsWith($safePrefix, [StringComparison]::OrdinalIgnoreCase)) { throw "unsafe upgrade workspace" }

try {
  New-Item -ItemType Directory -Path $resolvedRun -Force | Out-Null
  $workingData = Join-Path $resolvedRun "app-data"
  Copy-Item -LiteralPath $template -Destination $workingData -Recurse -Force
  $metadata = @{
    schemaVersion = 1
    baselineCommit = $BaselineCommit
    baselineInstaller = (Resolve-Path -LiteralPath $BaselineInstaller).Path
    baselineSha256 = $BaselineSha256.ToLowerInvariant()
    appDataTemplate = $template
    upgradeInstaller = (Resolve-Path -LiteralPath $UpgradeInstaller).Path
    upgradeSha256 = $UpgradeSha256.ToLowerInvariant()
  } | ConvertTo-Json -Depth 4
  [IO.File]::WriteAllText((Join-Path $resolvedRun "verification-inputs.json"), $metadata, [Text.UTF8Encoding]::new($false))
  Write-Host "Clean upgrade workspace prepared at $resolvedRun"
  Write-Host "Install the recorded baseline package, point it to $workingData, complete one login, then upgrade with the recorded upgrade package."
  Write-Host "This script validates immutable inputs and clean state; record the interactive results in the verification report."
} finally {
  if ($resolvedRun.StartsWith($safePrefix, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $resolvedRun)) {
    Remove-Item -LiteralPath $resolvedRun -Recurse -Force
  }
}
