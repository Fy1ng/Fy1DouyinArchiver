param(
  [Parameter(Mandatory = $true)][ValidateSet("Before", "After")][string]$Mode,
  [Parameter(Mandatory = $true)][string]$SnapshotPath
)

$ErrorActionPreference = "Stop"
if (Get-Process msedge, chrome -ErrorAction SilentlyContinue) {
  throw "close all ordinary Edge and Chrome processes before boundary verification"
}

function Get-TextHash([string]$Value) {
  $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
  $hash = [Security.Cryptography.SHA256]::HashData($bytes)
  return [Convert]::ToHexString($hash).ToLowerInvariant()
}

function Get-DirectorySummary([string]$Root) {
  if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
    return @{ root = [IO.Path]::GetFullPath($Root); exists = $false; digest = $null }
  }
  $resolved = (Resolve-Path -LiteralPath $Root).Path
  $rows = Get-ChildItem -LiteralPath $resolved -Recurse -Force -File | ForEach-Object {
    $relative = [IO.Path]::GetRelativePath($resolved, $_.FullName).Replace("\", "/")
    "$relative|$($_.Length)|$($_.LastWriteTimeUtc.Ticks)"
  } | Sort-Object
  return @{ root = $resolved; exists = $true; digest = Get-TextHash ($rows -join "`n") }
}

function Get-RegistrySummary([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) { return @{ path = $Path; exists = $false; values = @() } }
  $item = Get-ItemProperty -LiteralPath $Path
  $values = $item.PSObject.Properties |
    Where-Object { $_.Name -notmatch "^PS(Path|ParentPath|ChildName|Drive|Provider)$" } |
    ForEach-Object { @{ name = $_.Name; type = $_.Value.GetType().FullName; hash = Get-TextHash ([string]$_.Value) } } |
    Sort-Object name
  return @{ path = $Path; exists = $true; values = @($values) }
}

$snapshot = @{
  schemaVersion = 1
  profiles = @(
    Get-DirectorySummary (Join-Path $env:LOCALAPPDATA "Microsoft/Edge/User Data")
    Get-DirectorySummary (Join-Path $env:LOCALAPPDATA "Google/Chrome/User Data")
  )
  policies = @(
    Get-RegistrySummary "Registry::HKEY_CURRENT_USER\Software\Policies\Microsoft\Edge"
    Get-RegistrySummary "Registry::HKEY_LOCAL_MACHINE\Software\Policies\Microsoft\Edge"
    Get-RegistrySummary "Registry::HKEY_CURRENT_USER\Software\Policies\Google\Chrome"
    Get-RegistrySummary "Registry::HKEY_LOCAL_MACHINE\Software\Policies\Google\Chrome"
  )
}
$json = $snapshot | ConvertTo-Json -Depth 8 -Compress
$snapshotFile = [IO.Path]::GetFullPath($SnapshotPath)
if ($Mode -eq "Before") {
  $parent = Split-Path -Parent $snapshotFile
  if ($parent) { New-Item -ItemType Directory -Path $parent -Force | Out-Null }
  [IO.File]::WriteAllText($snapshotFile, $json, [Text.UTF8Encoding]::new($false))
  Write-Host "Browser boundary snapshot created."
  exit 0
}
if (-not (Test-Path -LiteralPath $snapshotFile -PathType Leaf)) { throw "missing Before snapshot" }
$before = [IO.File]::ReadAllText($snapshotFile)
if ($before -ne $json) { throw "ordinary browser profile or policy boundary changed" }
Write-Host "Browser boundary snapshot is unchanged."
