param(
  [string]$ManifestPath = "src-tauri/Cargo.toml"
)

$ErrorActionPreference = "Stop"
$tests = @(
  "edge_visible_constraints",
  "edge_hidden_constraints",
  "chrome_visible_constraints",
  "chrome_hidden_constraints"
)

$listed = (& cargo test --manifest-path $ManifestPath --test cdp_system_browser -- --list 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) { throw "unable to list CDP system-browser tests" }
foreach ($test in $tests) {
  if ($listed -notmatch [regex]::Escape("${test}: test")) {
    throw "missing browser constraint test: $test"
  }
}

foreach ($test in $tests) {
  $output = (& cargo test --manifest-path $ManifestPath --test cdp_system_browser $test -- --ignored --exact --test-threads=1 2>&1 | Out-String)
  if ($LASTEXITCODE -ne 0) {
    Write-Host $output
    throw "browser constraint test failed: $test"
  }
  if ($output -notmatch "test result: ok\. 1 passed; 0 failed; 0 ignored") {
    Write-Host $output
    throw "browser constraint test did not execute exactly once: $test"
  }
}

Write-Host "Edge/Chrome visible/hidden constraint matrix passed."
