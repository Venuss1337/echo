# Builds every asset of a versioned ECHO release from a clean checkout and a
# staged, reviewed decoder. It never uploads or publishes anything: the release
# workflow (or a maintainer) attaches the output to a draft GitHub release.
#
# Output, in a fresh ASCII directory:
#   ECHO-<version>-windows-x64-setup.exe       production NSIS installer
#   ECHO-<version>-source.tar.gz               this commit (git archive)
#   ECHO-<version>-rust-crates.tar.gz          every crate the Windows build compiles
#   ECHO-<version>-js-packages.tar.gz          the npm packages bundled into the UI
#   ECHO-<version>-native-source.tar           decoder sources, recipes and build receipts
#   ECHO-<version>-license-election.tar.gz    decoder license choices and notice patches
#   ECHO-<version>-THIRD_PARTY_NOTICES.txt     application and renderer notices
#   ECHO-<version>-third-party-inventory.json  machine-readable component list
#   ECHO-<version>-release-manifest.json       commit, toolchain and input digests
#   SHA256SUMS.txt                              digests of all of the above
param(
    [Parameter(Mandatory)][string] $Root,
    [Parameter(Mandatory)][string] $ResourceRoot,
    [Parameter(Mandatory)][string] $Destination
)
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path "$PSScriptRoot/../..").ProviderPath
$ResourceRoot = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$Destination = [IO.Path]::GetFullPath($Destination)
if ($Destination -match '[^\x20-\x7e]' -or (Test-Path -LiteralPath $Destination)) {
    throw 'Choose a fresh ASCII output directory for Windows tar.exe'
}

Push-Location $repo
try {
    # A release is built from exactly one commit, with nothing else mixed in.
    $dirty = git status --porcelain --untracked-files=normal
    if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'Release packaging requires a clean checkout' }
    $commit = (git rev-parse HEAD).Trim()

    $version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
    $cargoVersion = (Select-String -LiteralPath src-tauri/Cargo.toml -Pattern '^version = "([^"]+)"' |
        Select-Object -First 1).Matches[0].Groups[1].Value
    $packageVersion = (Get-Content package.json -Raw | ConvertFrom-Json).version
    if ($version -cnotmatch '^\d+\.\d+\.\d+$' -or $cargoVersion -cne $version -or $packageVersion -cne $version) {
        throw "Version mismatch: tauri $version, Cargo $cargoVersion, package.json $packageVersion"
    }
    $name = "ECHO-$version"

    # Only the reviewed callback-only decoder may ship.
    $files = Get-Content -LiteralPath "$ResourceRoot/dlls.json" -Raw | ConvertFrom-Json -AsHashtable
    $reviewed = Get-Content -LiteralPath "$PSScriptRoot/evidence/local/dlls.json" -Raw | ConvertFrom-Json -AsHashtable
    if ($files.Count -ne $reviewed.Count -or ($reviewed.Keys | Where-Object { $files[$_] -cne $reviewed[$_] })) {
        throw 'A release requires the complete reviewed local decoder manifest'
    }
    foreach ($evidence in 'inventory.json', 'source-inventory.json') {
        $expected = (Get-Content "$PSScriptRoot/evidence/local/$evidence" -Raw) -replace '\r\n', "`n"
        $staged = (Get-Content "$ResourceRoot/$evidence" -Raw) -replace '\r\n', "`n"
        if ($staged -cne $expected) { throw "Staged decoder differs from the reviewed $evidence" }
    }
    if (!(Test-Path -LiteralPath "$ResourceRoot/notices/README.txt" -PathType Leaf)) {
        throw 'Collect the decoder notices before packaging'
    }

    New-Item -ItemType Directory -Path $Destination | Out-Null
    $work = Join-Path $Destination '.work'
    New-Item -ItemType Directory -Path $work | Out-Null

    # Application and renderer notices, shipped inside the installer as well.
    $appNotices = Join-Path $work 'app-notices'
    bun scripts/collect-app-notices.ts $appNotices
    if ($LASTEXITCODE -ne 0) { throw 'Application notice collection failed' }

    $resources = [ordered]@{}
    foreach ($dll in $files.Keys) {
        if ($dll -cnotmatch '^[a-z0-9_.-]+\.dll$') { throw "Invalid DLL name: $dll" }
        $path = Join-Path $ResourceRoot "native/$dll"
        if ((Get-FileHash -LiteralPath $path).Hash -ne $files[$dll]) { throw "Decoder digest mismatch: $dll" }
        $resources[$path] = "native/$dll"
    }
    $notices = Join-Path $ResourceRoot 'notices'
    foreach ($file in (Get-ChildItem -LiteralPath $notices -File -Recurse)) {
        $target = [IO.Path]::GetRelativePath($notices, $file.FullName).Replace('\', '/')
        $resources[$file.FullName] = "notices/native/$target"
    }
    # The license choices made for the decoder's statically linked libraries
    # and the upstream texts they rely on travel with the decoder notices.
    $election = Join-Path $PSScriptRoot 'license-election'
    $resources["$election/LICENSE-ELECTION.txt"] = 'notices/native/LICENSE-ELECTION.txt'
    foreach ($file in (Get-ChildItem -LiteralPath "$election/texts" -File)) {
        $resources[$file.FullName] = "notices/native/texts/$($file.Name)"
    }
    $resources["$appNotices/THIRD_PARTY_NOTICES.txt"] = 'notices/THIRD_PARTY_NOTICES.txt'
    $resources["$repo/LICENSE"] = 'LICENSE.txt'
    $offer = Join-Path $work 'SOURCE.txt'
    @"
ECHO $version is free software under the GNU General Public License,
version 3 or later (LICENSE.txt). It includes libmpv and FFmpeg, built from
source, and other components listed in notices/.

The complete corresponding source for this exact build, including the
decoder's sources, patches, build recipes and receipts, the Rust crates and
JavaScript packages compiled into it, and the scripts that build and install
it, is published with the release at:

  https://github.com/Venuss1337/echo/releases/tag/v$version

Source commit: $commit
"@ | Set-Content -LiteralPath $offer -Encoding utf8
    $resources[$offer] = 'SOURCE.txt'

    $overlay = [ordered]@{
        mainBinaryName = 'echo'
        bundle = @{
            active = $true
            targets = @('nsis')
            resources = $resources
            windows = @{ nsis = @{ installMode = 'currentUser' } }
        }
    }
    $config = Join-Path $work 'tauri-release.json'
    $overlay | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $config
    $previous = $env:ECHO_NATIVE_DLL_MANIFEST
    try {
        $env:ECHO_NATIVE_DLL_MANIFEST = Join-Path $ResourceRoot 'dlls.json'
        bun run tauri build --config $config --features native-resources -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release installer build failed' }
    } finally {
        $env:ECHO_NATIVE_DLL_MANIFEST = $previous
    }
    $installer = Get-ChildItem src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis -Filter "ECHO_${version}_x64-setup.exe"
    if (@($installer).Count -ne 1) { throw 'Expected exactly one release installer' }
    Copy-Item -LiteralPath $installer.FullName -Destination "$Destination/$name-windows-x64-setup.exe"

    # Corresponding source. Everything below is derived from the lockfiles of
    # this commit, so the archives describe exactly what the installer holds.
    git archive --format=tar.gz --prefix="$name/" -o "$Destination/$name-source.tar.gz" $commit
    if ($LASTEXITCODE -ne 0) { throw 'Cannot archive the source commit' }

    # Cargo resolves the complete lockfile even for a Windows release build.
    # Keep dev and other-platform crates so an offline checkout can resolve it.
    $crates = Join-Path $work "$name-rust-crates"
    cargo vendor --locked --versioned-dirs --manifest-path src-tauri/Cargo.toml $crates | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'cargo vendor failed' }
    $source = 'source.vendored-sources.directory="' + $crates.Replace('\', '/') + '"'
    cargo --config 'source.crates-io.replace-with="vendored-sources"' --config $source metadata `
        --locked --offline --format-version 1 --manifest-path src-tauri/Cargo.toml | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'The vendored Rust source cannot resolve the locked graph offline' }
    @'
These are the exact crates.io sources in ECHO's Cargo.lock. To resolve Rust
dependencies offline, extract them next to the source archive and add to
src-tauri/.cargo/config.toml (invoke Cargo from src-tauri):

[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "<path to this directory>"
'@ | Set-Content -LiteralPath "$crates/README.txt" -Encoding utf8
    tar -czf "$Destination/$name-rust-crates.tar.gz" -C $work "$name-rust-crates"
    if ($LASTEXITCODE -ne 0) { throw 'Cannot archive the Rust crates' }

    # JavaScript: the published packages Vite bundles into the interface.
    $inventory = Get-Content -LiteralPath "$appNotices/third-party-inventory.json" -Raw | ConvertFrom-Json
    $js = Join-Path $work "$name-js-packages"
    foreach ($package in $inventory | Where-Object ecosystem -eq 'npm') {
        $source = Join-Path $repo "node_modules/$($package.name)"
        $installed = Get-Content -LiteralPath "$source/package.json" -Raw | ConvertFrom-Json
        if ($installed.version -cne $package.version) { throw "Unexpected nested npm package $($package.name)" }
        $target = Join-Path $js "$($package.name)@$($package.version)"
        New-Item -ItemType Directory -Force -Path (Split-Path $target) | Out-Null
        Copy-Item -LiteralPath $source -Destination $target -Recurse
        Remove-Item -LiteralPath "$target/node_modules" -Recurse -Force -ErrorAction SilentlyContinue
    }
    tar -czf "$Destination/$name-js-packages.tar.gz" -C $work "$name-js-packages"
    if ($LASTEXITCODE -ne 0) { throw 'Cannot archive the JavaScript packages' }

    # Native decoder: upstream archives, MSYS2 sources and recipes, build
    # receipts and notices, checked against the staged inventories.
    $native = Join-Path $work 'native-source'
    & "$PSScriptRoot/package-sources.ps1" -Root $Root -ResourceRoot $ResourceRoot -SourceCache "$Root/package-sources" -Destination $native | Out-Null
    Move-Item -LiteralPath "$native.tar" -Destination "$Destination/$name-native-source.tar"

    # The election statement, its notice patches and the texts it cites.
    tar -czf "$Destination/$name-license-election.tar.gz" -C $PSScriptRoot license-election
    if ($LASTEXITCODE -ne 0) { throw 'Cannot archive the license election' }

    Copy-Item -LiteralPath "$appNotices/THIRD_PARTY_NOTICES.txt" -Destination "$Destination/$name-THIRD_PARTY_NOTICES.txt"
    Copy-Item -LiteralPath "$appNotices/third-party-inventory.json" -Destination "$Destination/$name-third-party-inventory.json"

    $tools = [ordered]@{
        rustc = (rustc --version).Trim()
        cargo = (cargo --version).Trim()
        bun = (bun --version).Trim()
        tauri = (bun x tauri --version).Trim()
    }
    [ordered]@{
        product = 'ECHO'
        version = $version
        commit = $commit
        target = 'x86_64-pc-windows-msvc'
        decoder = $files
        tools = $tools
    } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$Destination/$name-release-manifest.json" -Encoding utf8

    Remove-Item -LiteralPath $work -Recurse -Force
    # LF line endings and no BOM, so `sha256sum -c` works on any platform.
    $sums = Get-ChildItem -LiteralPath $Destination -File | Where-Object Name -ne 'SHA256SUMS.txt' | Sort-Object Name | ForEach-Object {
        "$((Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant())  $($_.Name)"
    }
    [IO.File]::WriteAllText("$Destination/SHA256SUMS.txt", ($sums -join "`n") + "`n")
    Get-Content -LiteralPath "$Destination/SHA256SUMS.txt"
} finally {
    Pop-Location
}
