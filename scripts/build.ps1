$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
Push-Location (Join-Path $projectDir 'frontend')
try {
    npm ci
    if ($LASTEXITCODE -ne 0) { throw 'npm ci failed' }
    npm run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
} finally {
    Pop-Location
}
Push-Location $projectDir
try {
    cargo build --locked --release @args
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
} finally {
    Pop-Location
}
