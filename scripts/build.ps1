$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
Push-Location $projectDir
try {
    cargo build --locked --release @args
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
} finally {
    Pop-Location
}
