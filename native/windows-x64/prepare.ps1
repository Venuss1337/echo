# Fetch the exact source and toolchain archives into a new work directory.
param([Parameter(Mandatory)][string] $Root)
$ErrorActionPreference = 'Stop'
$Root = [IO.Path]::GetFullPath($Root)
if (Test-Path -LiteralPath $Root) { throw 'Choose a new work directory' }
New-Item -ItemType Directory -Path "$Root/packages" | Out-Null

function Get-PinnedFile($Url, $Path, $Digest) {
    Invoke-WebRequest -Uri $Url -OutFile $Path -TimeoutSec 180
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Digest) {
        throw "SHA-256 mismatch for $Url"
    }
}

$base = 'msys2-base-x86_64-20261120.tar.xz'
Get-PinnedFile "https://github.com/msys2/msys2-installer/releases/download/2026-11-20/$base" "$Root/$base" 'bd267f65e3656706d5e3b27d4f7d6c498e8ee3a54fcc3be200b7079bfc01f74a'
Get-PinnedFile 'https://ffmpeg.org/releases/ffmpeg-9.1.tar.xz' "$Root/ffmpeg-9.1.tar.xz" '1a4d394ae63d190a526cd052eb4415c156b5c5e2df37efab41a7de0b8fde2433'
Get-PinnedFile 'https://github.com/mpv-player/mpv/archive/b0841e8c4d156b8403dc1876c4cbd9011eb583f3.tar.gz' "$Root/mpv.tar.gz" 'a4a79fa6ca728762ac407734b0f93dbcbba250c3bb3bf97b77a342042be802e4'
Get-PinnedFile 'https://code.videolan.org/videolan/libplacebo/-/archive/v7.387.0/libplacebo-v7.387.0.tar.gz' "$Root/libplacebo-v7.387.0.tar.gz" '3bf5d72786682c194387c33d563872f6bfe202a9816f9dde6535a31a9b2e0d74'

# These are signed MSYS2 toolchain/dependency packages. mpv and FFmpeg are
# built from the upstream source archives above.
$lockedPackages = Get-Content "$PSScriptRoot/packages.sha256"
$lockedPackages | ForEach-Object -Parallel {
    $hash, $path = $_ -split '  ', 2
    $name = Split-Path $path -Leaf
    $package = Join-Path $using:Root "packages/$name"
    Invoke-WebRequest -Uri "https://repo.msys2.org/$path" -OutFile $package -TimeoutSec 180 -ErrorAction Stop
    if ((Get-FileHash -LiteralPath $package).Hash -ne $hash) { throw "SHA-256 mismatch for $name" }
    Invoke-WebRequest -Uri "https://repo.msys2.org/$path.sig" -OutFile "$package.sig" -TimeoutSec 180 -ErrorAction Stop
} -ThrottleLimit 8
foreach ($entry in $lockedPackages) {
    $hash, $path = $entry -split '  ', 2
    $package = Join-Path $Root "packages/$(Split-Path $path -Leaf)"
    if (!(Test-Path -LiteralPath "$package.sig" -PathType Leaf) -or
        (Get-FileHash -LiteralPath $package).Hash -ne $hash) {
        throw "Incomplete verified toolchain download: $path"
    }
}

tar -xf "$Root/$base" -C $Root
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract MSYS2 base' }
$configuration = "$Root/msys64/etc/pacman.conf"
$pacman = Get-Content -LiteralPath $configuration -Raw
# Require signatures even when installing from the local, hash-pinned cache.
$pacman = $pacman -replace '(?m)^#?LocalFileSigLevel\s*=.*$', 'LocalFileSigLevel = Required'
if ($pacman -notmatch '(?m)^LocalFileSigLevel = Required$') { throw 'Cannot require local signatures' }
[IO.File]::WriteAllText($configuration, $pacman)

$env:MSYSTEM = 'UCRT64'
$env:CHERE_INVOKING = '1'
$bash = "$Root/msys64/usr/bin/bash.exe"
& $bash -l -c 'set -eu; cd "$(cygpath -u "$1")/packages"; pacman -U --noconfirm --needed ./*.pkg.tar.zst' echo $Root
if ($LASTEXITCODE -ne 0) { throw 'Cannot install pinned MSYS2 packages' }
Copy-Item -LiteralPath "$PSScriptRoot/build.sh" -Destination "$Root/build.sh"
