# Assemble an offline native source review package from a staged candidate.
param(
    [Parameter(Mandatory)][string] $Root,
    [Parameter(Mandatory)][string] $ResourceRoot,
    [Parameter(Mandatory)][string] $SourceCache,
    [Parameter(Mandatory)][string] $Destination
)
$ErrorActionPreference = 'Stop'
$Destination = [IO.Path]::GetFullPath($Destination)
if ($Destination -match '[^\x20-\x7e]' -or (Test-Path -LiteralPath $Destination) -or
    (Test-Path -LiteralPath "$Destination.tar")) {
    throw 'Choose a fresh ASCII output directory for Windows tar.exe'
}
$inventory = Get-Content -LiteralPath "$ResourceRoot/inventory.json" -Raw | ConvertFrom-Json -AsHashtable
$sources = Get-Content -LiteralPath "$ResourceRoot/source-inventory.json" -Raw | ConvertFrom-Json
$expected = @($inventory.packages.Keys | Sort-Object)
$covered = @($sources.packages | Sort-Object)
if (Compare-Object $expected $covered -CaseSensitive) {
    throw 'Package source inventory differs from linked package set'
}
New-Item -ItemType Directory -Path "$Destination/upstream","$Destination/msys2","$Destination/evidence","$Destination/recipes" | Out-Null

function Copy-VerifiedSource($Directory, $Name, $Digest, $Target) {
    if ($Name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._+-]+$' -or $Digest -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Invalid source archive record'
    }
    $file = Join-Path $Directory $Name
    if ((Get-FileHash -LiteralPath $file).Hash.ToLowerInvariant() -cne $Digest) {
        throw "Source archive hash mismatch: $Name"
    }
    Copy-Item -LiteralPath $file -Destination $Target
}

# Read the three upstream archive digests from the same lock used before build.
$recipe = Get-Content -LiteralPath "$PSScriptRoot/build.sh"
$pins = $recipe | Select-String -CaseSensitive '^([a-f0-9]{64})  ([a-z0-9.-]+\.tar\.(?:gz|xz))$'
if (@($pins).Count -ne 3) { throw 'Unexpected upstream source archive lock' }
foreach ($pin in $pins) {
    Copy-VerifiedSource $Root $pin.Matches[0].Groups[2].Value $pin.Matches[0].Groups[1].Value "$Destination/upstream"
}
foreach ($source in $sources) {
    if ($source.status -cne 'recipe-matches-binary-metadata') { throw "Unverified recipe: $($source.url)" }
    foreach ($name in $source.packages) {
        if ($source.url -cne $inventory.packages[$name].source_archive -or
            $source.pkgbuild_sha256 -cne $inventory.packages[$name].pkgbuild_sha256[0]) {
            throw "Source recipe evidence differs from the linked component: $name"
        }
    }
    Copy-VerifiedSource $SourceCache (Split-Path $source.url -Leaf) $source.archive_sha256 "$Destination/msys2"
}

$receipts = @(
    'dlls.json', 'inventory.json', 'source-inventory.json', 'libmpv-link.map',
    'config.log', 'ffmpeg-config.h', 'ffmpeg-components.h', 'ffmpeg-config.mak',
    'libplacebo-options.json', 'mpv-options.json'
)
foreach ($name in $receipts) {
    Copy-Item -LiteralPath "$ResourceRoot/$name" -Destination "$Destination/evidence"
}
Copy-Item -LiteralPath "$ResourceRoot/notices" -Destination $Destination -Recurse
Copy-Item -LiteralPath "$ResourceRoot/package-build-info" -Destination "$Destination/evidence" -Recurse
Get-ChildItem -LiteralPath $PSScriptRoot -File | Copy-Item -Destination "$Destination/recipes"
$repo = (Resolve-Path "$PSScriptRoot/../..").ProviderPath
Copy-Item -LiteralPath "$repo/docs/NATIVE.md","$repo/LICENSE" -Destination $Destination
Get-ChildItem -LiteralPath $Destination -File -Recurse | Sort-Object FullName | ForEach-Object {
    $relative = [IO.Path]::GetRelativePath($Destination, $_.FullName).Replace('\', '/')
    "$((Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant())  $relative"
} | Set-Content -LiteralPath "$Destination/SHA256SUMS" -Encoding utf8

& tar -cf "$Destination.tar" -C $Destination .
if ($LASTEXITCODE -ne 0) { throw 'Cannot create native source review archive' }
Get-FileHash -LiteralPath "$Destination.tar"
