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

$installationJson = @(& $vswhere -all -prerelease -products '*' -format json -utf8)
$vswhereExitCode = $LASTEXITCODE
if ($vswhereExitCode -ne 0) {
    throw "Cannot discover Visual Studio installations: vswhere exit code $vswhereExitCode."
}
try {
    $installations = @(($installationJson -join [Environment]::NewLine) | ConvertFrom-Json)
} catch {
    throw "Cannot parse Visual Studio installations: vswhere exit code $vswhereExitCode; $($_.Exception.Message)"
}
Write-Output "Visual Studio installation count: $($installations.Count); vswhere exit code: $vswhereExitCode."

# Query actual files instead of assuming a particular component ID or stable VS release.
# Capture the complete native output before filtering, so the exit code belongs to vswhere.
$dumpbinCandidates = @(
    foreach ($installation in $installations) {
        Write-Host "Visual Studio installation: $($installation.installationPath)"
        $toolsRoot = Join-Path $installation.installationPath 'VC\Tools\MSVC'
        if (-not (Test-Path -LiteralPath $toolsRoot -PathType Container)) { continue }
        foreach ($toolsVersion in @(Get-ChildItem -LiteralPath $toolsRoot -Directory)) {
            $candidate = Join-Path $toolsVersion.FullName 'bin\Hostx64\x64\dumpbin.exe'
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { $candidate }
        }
    }
)
Write-Output "x64 dumpbin candidate count: $($dumpbinCandidates.Count)."
$dumpbinCandidates | ForEach-Object { Write-Output "x64 dumpbin candidate: $_" }
if (-not $dumpbinCandidates.Count) {
    throw "Cannot verify Windows runtime dependencies: no x64 dumpbin.exe found in $($installations.Count) installations; vswhere exit code $vswhereExitCode."
}
$dumpbin = $dumpbinCandidates | Sort-Object -Descending | Select-Object -First 1
Write-Output "Dependency inspector: $dumpbin"
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
