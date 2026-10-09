param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)

$ErrorActionPreference = 'Stop'
$binaryPath = (Resolve-Path -LiteralPath $Binary).ProviderPath
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere)) {
    throw 'Cannot verify Windows runtime dependencies: vswhere.exe was not found.'
}

$installationPath = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1
if ($LASTEXITCODE -ne 0 -or -not $installationPath) {
    throw 'Cannot verify Windows runtime dependencies: Visual Studio C++ tools were not found.'
}
$toolsRoot = Join-Path $installationPath 'VC\Tools\MSVC'
$toolsVersion = Get-ChildItem -LiteralPath $toolsRoot -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin\Hostx64\x64\dumpbin.exe') } |
    Sort-Object { [version]$_.Name } -Descending |
    Select-Object -First 1
if (-not $toolsVersion) {
    throw 'Cannot verify Windows runtime dependencies: x64 dumpbin.exe was not found.'
}
$dumpbin = Join-Path $toolsVersion.FullName 'bin\Hostx64\x64\dumpbin.exe'
$dependencies = & $dumpbin /DEPENDENTS $binaryPath
if ($LASTEXITCODE -ne 0) {
    throw "dumpbin failed while checking $binaryPath."
}
$dependencies | Write-Output

$forbidden = '(?i)^\s*(vcruntime[^\s]*|msvcp[^\s]*|concrt[^\s]*|ucrtbase|api-ms-win-crt[^\s]*)\.dll\s*$'
$runtimeDependencies = @($dependencies | Where-Object { $_ -match $forbidden })
if ($runtimeDependencies.Count -gt 0) {
    $names = ($runtimeDependencies | ForEach-Object { $_.Trim() }) -join ', '
    throw "The executable still depends on a dynamic C/C++ runtime: $names. Expected target-feature=+crt-static."
}
Write-Output 'Verified: no direct MSVC/UCRT DLL dependency was found.'
