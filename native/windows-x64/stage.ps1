# Stage a build candidate and inventory its actual PE import closure.
param(
    [Parameter(Mandatory)][string] $Root,
    [Parameter(Mandatory)][string] $Destination,
    [switch] $LocalOnly
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path -LiteralPath $Root).ProviderPath
$Destination = [IO.Path]::GetFullPath($Destination)
if (Test-Path -LiteralPath $Destination) { throw 'Choose a fresh staging directory' }
$build = Join-Path $Root 'build/mpv-build'
$tools = Join-Path $Root 'msys64/ucrt64/bin'
$pacman = Join-Path $Root 'msys64/usr/bin/pacman.exe'
$objdump = Join-Path $tools 'objdump.exe'
$candidate = Join-Path $build 'libmpv-2.dll'
$ffmpegConfig = Join-Path $Root 'build/ffmpeg-9.1/config.h'
$licenseConfig = Get-Content -LiteralPath $ffmpegConfig
foreach ($line in '#define CONFIG_GPL 1','#define CONFIG_VERSION3 1','#define CONFIG_NONFREE 0',
    '#define FFMPEG_LICENSE "GPL version 3 or later"') {
    if ($line -cnotin $licenseConfig) { throw "Unexpected FFmpeg license configuration: expected $line" }
}
if ($LocalOnly) {
    $components = Get-Content -LiteralPath "$Root/build/ffmpeg-9.1/config_components.h"
    if ('#define CONFIG_NETWORK 0' -cnotin $licenseConfig -or
        ($components -cmatch '^#define CONFIG_.*_PROTOCOL 1$')) {
        throw 'The local decoder must have no FFmpeg network support or URL protocols'
    }
}
$pending = [Collections.Generic.Queue[string]]::new()
$pending.Enqueue($candidate)
$records = [ordered]@{}
$system = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$packages = [ordered]@{}
$packageLock = Get-Content "$PSScriptRoot/packages.sha256"
$buildInfoDirectory = Join-Path $Destination 'package-build-info'
New-Item -ItemType Directory -Path $buildInfoDirectory | Out-Null

function Get-Owner($File) {
    $relative = [IO.Path]::GetRelativePath("$Root/msys64", [IO.Path]::GetFullPath($File)).Replace('\', '/')
    if (!$relative.StartsWith('ucrt64/')) { throw "Unknown link input: $File" }
    $package = & $pacman -Qoq "/$relative"
    if ($LASTEXITCODE -ne 0 -or !$package) { throw "Unowned dependency: $relative" }
    $package = $package.Trim()
    if (!$packages.Contains($package)) {
        $version = ((& $pacman -Q $package) -split ' ', 2)[1].Replace(':', '~')
        $archive = @(Get-ChildItem "$Root/msys64/var/cache/pacman/pkg","$Root/packages" -Filter "$package-$version-*.pkg.tar.zst" -ErrorAction SilentlyContinue)[0]
        if (!$archive) { throw "Missing cached package: $package" }
        $archivePath = "mingw/ucrt64/$($archive.Name)"
        $archiveHash = (Get-FileHash -LiteralPath $archive.FullName).Hash.ToLowerInvariant()
        if ("$archiveHash  $archivePath" -cnotin $packageLock) { throw "Package not pinned: $archivePath" }
        $entries = & tar -xOf $archive.FullName .PKGINFO
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect package metadata: $package" }
        $buildInfo = & tar -xOf $archive.FullName .BUILDINFO
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect package build info: $package" }
        $buildInfo | Set-Content -LiteralPath "$buildInfoDirectory/$package.txt" -Encoding utf8
        $base = ($entries -match '^pkgbase = ' -replace '^pkgbase = ')[0]
        $packages[$package] = [ordered]@{
            version = $version
            archive = "https://repo.msys2.org/$archivePath"
            archive_sha256 = $archiveHash
            upstream = @($entries -match '^url = ' -replace '^url = ')
            declared_licenses = @($entries -match '^license = ' -replace '^license = ')
            packager = @($entries -match '^packager = ' -replace '^packager = ')
            pkgbuild_sha256 = @($buildInfo -match '^pkgbuild_sha256sum = ' -replace '^pkgbuild_sha256sum = ')
            source_archive = "https://repo.msys2.org/mingw/sources/$base-$version.src.tar.zst"
        }
    }
    $package
}

while ($pending.Count) {
    $file = $pending.Dequeue()
    $name = (Split-Path $file -Leaf).ToLowerInvariant()
    if ($records.Contains($name)) { continue }
    $headers = & $objdump -p $file
    if ($LASTEXITCODE -ne 0 -or ($headers -join "`n") -notmatch 'file format pei-x86-64') {
        throw "Invalid Windows x64 PE: $file"
    }
    $imports = @($headers | ForEach-Object {
        if ($_ -match 'DLL Name:\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
    } | Sort-Object -Unique)
    if (!$imports) { throw "Cannot establish PE imports for $name" }
    $record = [ordered]@{
        sha256 = (Get-FileHash -LiteralPath $file).Hash.ToLowerInvariant()
        imports = $imports
    }
    if ($file -eq $candidate) {
        foreach ($flag in 'HIGH_ENTROPY_VA','DYNAMIC_BASE','NX_COMPAT') {
            if (!(($headers -join "`n") -match "\b$flag\b")) { throw "Missing PE hardening flag: $flag" }
        }
        $record.pe_hardening = @('HIGH_ENTROPY_VA', 'DYNAMIC_BASE', 'NX_COMPAT')
        $record.component = 'mpv v0.42.0; statically linked FFmpeg 9.1; see link map'
    } else {
        $record.package = Get-Owner $file
    }
    $records[$name] = $record
    foreach ($import in $imports) {
        $dependency = Join-Path $tools $import
        if ($import -match '^(api|ext)-ms-win-' -or (Test-Path -LiteralPath "$env:SystemRoot/System32/$import")) {
            if (Test-Path -LiteralPath $dependency) { throw "System/redistributable collision: $import" }
            $null = $system.Add($import)
        } elseif (Test-Path -LiteralPath $dependency -PathType Leaf) {
            $pending.Enqueue($dependency)
        } else {
            throw "Unresolved dependency: $import (imported by $name)"
        }
    }
}

$map = Join-Path $build 'libmpv-link.map'
$linkMap = Get-Content -LiteralPath $map
$archives = @($linkMap | ForEach-Object {
    if ($_ -cmatch '^([A-Za-z]:/[^()]+\.a)\(') { [IO.Path]::GetFullPath($Matches[1]) }
} | Sort-Object -Unique)
if (!$archives) { throw 'No linked archive members found in GNU ld map' }
$linked = foreach ($file in $archives) {
    $relative = [IO.Path]::GetRelativePath($Root, $file).Replace('\', '/')
    $owner = if ($relative -match '^msys64/echo-native/lib/lib(av|sw).*\.a$') {
        'FFmpeg 9.1 source build'
    } elseif ($relative -ceq 'msys64/echo-native/lib/libplacebo.a') {
        'libplacebo 7.387.0 source build'
    } else {
        Get-Owner $file
    }
    [ordered]@{
        archive = $relative
        sha256 = (Get-FileHash -LiteralPath $file).Hash.ToLowerInvariant()
        component = $owner
    }
}
$startupObjects = @($linkMap | ForEach-Object {
    if ($_ -cmatch '^LOAD ([A-Za-z]:/.*\.o)\s*$') { [IO.Path]::GetFullPath($Matches[1]) }
} | Sort-Object -Unique | ForEach-Object {
    [ordered]@{
        object = [IO.Path]::GetRelativePath($Root, $_).Replace('\', '/')
        sha256 = (Get-FileHash -LiteralPath $_).Hash.ToLowerInvariant()
        component = Get-Owner $_
    }
})
if (!$startupObjects) { throw 'No toolchain startup objects found in GNU ld map' }
if ('libplacebo 7.387.0 source build' -cnotin $linked.component -or
    $linked.archive -match '/libdovi(?:\.dll)?\.a$' -or
    $linkMap -match '\.rcgu\.o\)|\b(?:__rust_alloc|rust_eh_personality)\b' -or
    $records.Keys -match '^(?:lib)?dovi.*\.dll$') {
    throw 'Expected source-built libplacebo without libdovi or its Rust runtime in the link closure'
}

$native = Join-Path $Destination 'native'
New-Item -ItemType Directory -Path $native | Out-Null
foreach ($name in $records.Keys) {
    $from = if ($name -eq 'libmpv-2.dll') { $candidate } else { Join-Path $tools $name }
    Copy-Item -LiteralPath $from -Destination (Join-Path $native $name)
}
$manifest = [ordered]@{}
foreach ($name in ($records.Keys | Sort-Object)) { $manifest[$name] = $records[$name].sha256 }
$manifest | ConvertTo-Json | Set-Content "$Destination/dlls.json"
[ordered]@{
    architecture = 'x86_64-w64-mingw32 (UCRT)'
    dlls = $records
    system_imports = @($system | Sort-Object)
    linked_archives = @($linked)
    startup_objects = $startupObjects
    packages = $packages
} |
    ConvertTo-Json -Depth 8 | Set-Content "$Destination/inventory.json"
Copy-Item -LiteralPath $map,"$Root/build/ffmpeg-9.1/ffbuild/config.log" -Destination $Destination
Copy-Item -LiteralPath $ffmpegConfig -Destination "$Destination/ffmpeg-config.h"
Copy-Item -LiteralPath "$Root/build/ffmpeg-9.1/config_components.h" -Destination "$Destination/ffmpeg-components.h"
Copy-Item -LiteralPath "$Root/build/ffmpeg-9.1/ffbuild/config.mak" -Destination "$Destination/ffmpeg-config.mak"
Copy-Item -LiteralPath "$Root/build/placebo-build/meson-info/intro-buildoptions.json" -Destination "$Destination/libplacebo-options.json"
Copy-Item -LiteralPath "$Root/build/mpv-build/meson-info/intro-buildoptions.json" -Destination "$Destination/mpv-options.json"
