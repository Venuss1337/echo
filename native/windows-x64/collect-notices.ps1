# Assemble primary component license texts for a private build review.
param(
    [Parameter(Mandatory)][string] $Root,
    [Parameter(Mandatory)][string] $ResourceRoot,
    [Parameter(Mandatory)][string] $SourceCache
)
$ErrorActionPreference = 'Stop'
$inventory = Get-Content "$ResourceRoot/inventory.json" -Raw | ConvertFrom-Json -AsHashtable
$output = Join-Path $ResourceRoot 'notices'
if (Test-Path -LiteralPath $output) { throw 'Notices directory must be fresh' }
New-Item -ItemType Directory -Path $output | Out-Null

# MSYS2's libass binary package does not install its license text. Recover
# COPYING from the matching, checksum-recorded corresponding-source archive.
$libass = @((Get-Content "$ResourceRoot/source-inventory.json" -Raw | ConvertFrom-Json).Where({
    $_.packages -contains 'mingw-w64-ucrt-x86_64-libass'
}))[0]
if ($libass.status -ne 'recipe-matches-binary-metadata') { throw 'libass source not verified' }
$archive = Join-Path $SourceCache (Split-Path $libass.url -Leaf)
if ((Get-FileHash -LiteralPath $archive).Hash -ne $libass.archive_sha256) { throw 'libass source changed' }
$scratch = Join-Path $SourceCache ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $scratch | Out-Null
tar -xf $archive -C $scratch mingw-w64-libass/libass-0.17.5.tar.xz
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract libass source' }
tar -xf "$scratch/mingw-w64-libass/libass-0.17.5.tar.xz" -C $scratch libass-0.17.5/COPYING
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract libass notice' }

foreach ($package in $inventory.packages.Keys) {
    if ($package -notmatch '^mingw-w64-ucrt-x86_64-([a-z0-9+-]+)$') { throw 'Invalid package in inventory' }
    $name = $Matches[1]
    $target = Join-Path $output $name
    New-Item -ItemType Directory -Path $target | Out-Null
    $files = if ($name -eq 'libass') {
        Get-Item -LiteralPath "$scratch/libass-0.17.5/COPYING"
    } else {
        Get-ChildItem -LiteralPath "$Root/msys64/ucrt64/share/licenses/$name" -File -Recurse
    }
    if (!$files) { throw "Missing license texts: $name" }
    foreach ($file in $files) { Copy-Item -LiteralPath $file.FullName -Destination $target }
}

$mpv = Join-Path $Root 'build/mpv-b0841e8c4d156b8403dc1876c4cbd9011eb583f3'
$ffmpeg = Join-Path $Root 'build/ffmpeg-9.1'
New-Item -ItemType Directory -Path "$output/mpv","$output/ffmpeg","$output/libplacebo" | Out-Null
Copy-Item -LiteralPath "$Root/build/libplacebo-v7.387.0/LICENSE" -Destination "$output/libplacebo"
Get-ChildItem -LiteralPath $mpv -File | Where-Object Name -Match '^(Copyright|LICENSE\.)' |
    Copy-Item -Destination "$output/mpv"
Get-ChildItem -LiteralPath $ffmpeg -File | Where-Object Name -Match '^(COPYING\.|LICENSE\.md$)' |
    Copy-Item -Destination "$output/ffmpeg"
@'
ECHO Windows native candidate — private verification only.

These primary license texts accompany the binaries for review. See the native
artifact record and component inventory for source origins and build details.
Nested static dependency notices and redistribution review are pending.
'@ | Set-Content -LiteralPath "$output/README.txt"
