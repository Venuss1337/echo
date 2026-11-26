# Exercises the actual callback-only decoder in a headless, null-output test process.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [switch] $Measure,
    [ValidateRange(60, 600)][int] $DurationSeconds = 60
)
$ErrorActionPreference = 'Stop'
$previous = $env:ECHO_LIVE_RESOURCE_ROOT
$previousDuration = $env:ECHO_LIVE_MEASURE_SECONDS
$env:ECHO_LIVE_RESOURCE_ROOT = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$env:ECHO_LIVE_MEASURE_SECONDS = if ($Measure) { $DurationSeconds.ToString() } else { $null }
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    bun run fixtures:audio
    if ($LASTEXITCODE -ne 0) { throw 'Audio fixture generation failed' }
    $test = if ($Measure) { 'native_live_measurement' } else { 'native_live_radio' }
    cargo test --locked --manifest-path src-tauri/Cargo.toml --lib $test -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Live native radio acceptance failed' }
} finally {
    Pop-Location
    $env:ECHO_LIVE_RESOURCE_ROOT = $previous
    $env:ECHO_LIVE_MEASURE_SECONDS = $previousDuration
}
