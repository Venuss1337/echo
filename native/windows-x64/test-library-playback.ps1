# Runs the library and playback coordinators together against the staged,
# reviewed native DLLs with the null audio output. It proves that scanning does
# not disturb playback and that a vanished file is reported back to the index.
# It does not produce audible sound and changes no audio device.
param(
    [Parameter(Mandatory)][string] $ResourceRoot
)
$ErrorActionPreference = 'Stop'
$previous = $env:ECHO_LOCAL_NATIVE_ROOT
$env:ECHO_LOCAL_NATIVE_ROOT = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    cargo test --locked --manifest-path src-tauri/Cargo.toml --lib library::native_tests -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Library and playback integration failed' }
} finally {
    Pop-Location
    [Environment]::SetEnvironmentVariable('ECHO_LOCAL_NATIVE_ROOT', $previous, 'Process')
}
