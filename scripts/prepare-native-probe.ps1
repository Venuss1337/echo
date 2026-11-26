# Explicit developer step. No app, build, or test invokes this downloader.
param([string] $Archive)
$ErrorActionPreference = 'Stop'

$asset = 'libmpv-v0.42.0-x86_64-w64-mingw32-gpl.zip'
$archiveHash = 'B99E0D183024AE479D1B36A93FA262C0297DFB025B99715B98DA377FFB6C2913'
$dllHash = '59D68B9670699B30D2C3D47BD29B1856DBDE92ADE136FA57711413A5C3E07B10'

# Stage just the DLL under the current user's profile, away from the checkout
# and common DLL lookup locations. Each invocation uses a fresh directory.
$root = Join-Path $env:LOCALAPPDATA ('ECHO/NativeProbe/' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
if (!$Archive) {
    $Archive = Join-Path $root $asset
    Invoke-WebRequest -Uri "https://github.com/mpv-player/mpv/releases/download/v0.42.0/$asset" -OutFile $Archive -TimeoutSec 180
}
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash -ne $archiveHash) {
    throw 'Unexpected libmpv archive digest'
}

$directory = Join-Path $root 'native'
New-Item -ItemType Directory -Path $directory | Out-Null
$dll = Join-Path $directory 'libmpv-2.dll'
$zip = [IO.Compression.ZipFile]::OpenRead((Resolve-Path -LiteralPath $Archive).ProviderPath)
try {
    $entry = $zip.GetEntry('libmpv-2.dll')
    if ($null -eq $entry) { throw 'Pinned DLL is missing from the archive' }
    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dll)
} finally {
    $zip.Dispose()
}

if ((Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash -ne $dllHash) {
    throw 'Unexpected extracted libmpv DLL digest'
}
Write-Output $dll
