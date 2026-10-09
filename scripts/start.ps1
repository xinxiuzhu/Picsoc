$ErrorActionPreference = 'Stop'
$projectDir = Split-Path -Parent $PSScriptRoot
$picsocBinary = if ($env:PICSOC_BIN) { $env:PICSOC_BIN } else { Join-Path $projectDir 'target/release/picsoc.exe' }
if (-not (Test-Path $picsocBinary -PathType Leaf)) {
    throw "找不到可执行程序：$picsocBinary。请先运行 scripts/build.ps1。"
}
Push-Location $projectDir
try {
    & $picsocBinary @args
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
