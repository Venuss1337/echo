# Uses the production loader in isolated workers, without starting the app UI.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [string] $Manifest
)
$ErrorActionPreference = 'Stop'
$ResourceRoot = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
if (!$Manifest) { $Manifest = Join-Path $ResourceRoot 'dlls.json' }
$Manifest = (Resolve-Path -LiteralPath $Manifest).ProviderPath
$previousRoot = $env:ECHO_INSTALLED_NATIVE_ROOT
$previousManifest = $env:ECHO_INSTALLED_NATIVE_MANIFEST
$env:ECHO_INSTALLED_NATIVE_ROOT = $ResourceRoot
$env:ECHO_INSTALLED_NATIVE_MANIFEST = $Manifest
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    bun run test:native
    if ($LASTEXITCODE -ne 0) { throw 'Installed native acceptance failed' }
} finally {
    Pop-Location
    $env:ECHO_INSTALLED_NATIVE_ROOT = $previousRoot
    $env:ECHO_INSTALLED_NATIVE_MANIFEST = $previousManifest
}
