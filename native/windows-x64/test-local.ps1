# Build test encodings from the repository's generated original WAV fixture.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [string] $Ffmpeg,
    [string] $LegacyResourceRoot,
    [string] $Manifest,
    [switch] $NoOutputEndpoint,
    [switch] $VerifyDefaultOutput,
    [switch] $Measure
)
$ErrorActionPreference = 'Stop'
if (@($NoOutputEndpoint, $VerifyDefaultOutput, $Measure).Where({ $_ }).Count -gt 1) {
    throw 'Choose at most one optional playback mode'
}
$previous = @{
    ECHO_LOCAL_NATIVE_ROOT = $env:ECHO_LOCAL_NATIVE_ROOT
    ECHO_LOCAL_NATIVE_MANIFEST = $env:ECHO_LOCAL_NATIVE_MANIFEST
    ECHO_LEGACY_NATIVE_ROOT = $env:ECHO_LEGACY_NATIVE_ROOT
    ECHO_LOCAL_NO_OUTPUT = $env:ECHO_LOCAL_NO_OUTPUT
    ECHO_LOCAL_DEFAULT_OUTPUT = $env:ECHO_LOCAL_DEFAULT_OUTPUT
    ECHO_MEASURE_LOCAL = $env:ECHO_MEASURE_LOCAL
    ECHO_LOCAL_NATIVE_FIXTURES = $env:ECHO_LOCAL_NATIVE_FIXTURES
}
$env:ECHO_LOCAL_NATIVE_ROOT = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$env:ECHO_LOCAL_NATIVE_MANIFEST = if ($Manifest) { (Resolve-Path -LiteralPath $Manifest).ProviderPath } else { $null }
$env:ECHO_LOCAL_NO_OUTPUT = if ($NoOutputEndpoint) { '1' } else { $null }
$env:ECHO_LOCAL_DEFAULT_OUTPUT = if ($VerifyDefaultOutput) { '1' } else { $null }
$env:ECHO_MEASURE_LOCAL = if ($Measure) { '1' } else { $null }
$env:ECHO_LEGACY_NATIVE_ROOT = if ($LegacyResourceRoot) { (Resolve-Path -LiteralPath $LegacyResourceRoot).ProviderPath } else { $null }
$fixtures = Join-Path ([IO.Path]::GetTempPath()) ('echo-local-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtures | Out-Null
$env:ECHO_LOCAL_NATIVE_FIXTURES = $fixtures
$encoder = if ($Ffmpeg) { (Resolve-Path -LiteralPath $Ffmpeg).ProviderPath } else { $null }
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    bun run fixtures:audio
    if ($LASTEXITCODE -ne 0) { throw 'Audio fixture generation failed' }
    Copy-Item -LiteralPath '.echo-fixtures/transport.wav' -Destination "$fixtures/tone.wav"
    $formats = [ordered]@{ mp3 = 'libmp3lame'; flac = 'flac'; ogg = 'libvorbis'; m4a = 'aac'; aac = 'aac' }
    if (!$NoOutputEndpoint -and !$VerifyDefaultOutput -and !$Measure) {
        foreach ($entry in $formats.GetEnumerator()) {
            $filename = "tone.$($entry.Key)"
            if ($encoder) {
                & $encoder -hide_banner -nostdin -loglevel error -i "$fixtures/tone.wav" -fflags +bitexact -flags:a +bitexact -c:a $entry.Value "$fixtures/$filename"
                if ($LASTEXITCODE -ne 0) { throw "Could not encode $($entry.Key) fixture" }
            } else {
                Copy-Item -LiteralPath "tests/fixtures/audio/$filename" -Destination $fixtures
            }
        }
    }
    cargo test --locked --manifest-path src-tauri/Cargo.toml --test local_decoder -- --ignored --exact local_decoder --nocapture
    if ($LASTEXITCODE -ne 0) { throw 'Local decoder acceptance failed' }
} finally {
    Pop-Location
    foreach ($name in $previous.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
    }
    # Delete only known files in the newly created staging directory.
    Get-ChildItem -LiteralPath $fixtures -File | Remove-Item
    Remove-Item -LiteralPath $fixtures
}
