# Builds a private NSIS candidate. It never uploads or publishes an installer.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [switch] $LocalOnly
)
$ErrorActionPreference = 'Stop'
$ResourceRoot = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$manifest = Join-Path $ResourceRoot 'dlls.json'
$files = Get-Content -LiteralPath $manifest -Raw | ConvertFrom-Json -AsHashtable
if (!$files.Contains('libmpv-2.dll')) { throw 'Incomplete native manifest' }
if ($LocalOnly) {
    $reviewed = Get-Content -LiteralPath "$PSScriptRoot/evidence/local/dlls.json" -Raw | ConvertFrom-Json -AsHashtable
    if ($files.Count -ne $reviewed.Count -or
        ($reviewed.Keys | Where-Object { $files[$_] -cne $reviewed[$_] })) {
        throw 'The M2 candidate requires the complete reviewed local DLL manifest'
    }
}
$resources = [ordered]@{}
foreach ($name in $files.Keys) {
    if ($name -cnotmatch '^[a-z0-9_.-]+\.dll$') { throw "Invalid DLL name: $name" }
    $path = Join-Path $ResourceRoot "native/$name"
    if ((Get-FileHash -LiteralPath $path).Hash -ne $files[$name]) {
        throw "Candidate resource digest mismatch: $name"
    }
    $resources[$path] = "native/$name"
}
$notices = Join-Path $ResourceRoot 'notices'
if (!(Test-Path -LiteralPath "$notices/README.txt" -PathType Leaf)) {
    throw 'Collect candidate notices before packaging'
}
foreach ($file in (Get-ChildItem -LiteralPath $notices -File -Recurse)) {
    $target = [IO.Path]::GetRelativePath($notices, $file.FullName).Replace('\', '/')
    $resources[$file.FullName] = "notices/$target"
}

# A distinct identity avoids touching any existing ECHO installation or data.
$overlay = [ordered]@{
    identifier = 'org.echo.player.nativecandidate'
    productName = 'ECHO Native Candidate'
    mainBinaryName = 'echo-native-candidate'
    bundle = @{ active = $true; targets = @('nsis'); resources = $resources; windows = @{ nsis = @{ installMode = 'currentUser' } } }
}
if ($LocalOnly) {
    $overlay.identifier = 'org.echo.player.localcandidate'
    $overlay.productName = 'ECHO Local Candidate'
    $overlay.mainBinaryName = 'echo-local-candidate'
}
$config = Join-Path $ResourceRoot 'tauri-candidate.json'
$overlay | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $config
$previous = $env:ECHO_NATIVE_DLL_MANIFEST
try {
    $env:ECHO_NATIVE_DLL_MANIFEST = $manifest
    Push-Location (Resolve-Path "$PSScriptRoot/../..")
    try {
        bun run tauri build --config $config --features native-resources -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Candidate installer build failed' }
    } finally {
        Pop-Location
    }
} finally {
    $env:ECHO_NATIVE_DLL_MANIFEST = $previous
}
