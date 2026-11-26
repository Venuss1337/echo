# Use upstream release archives with pinned GitHub release hashes.
$ErrorActionPreference = 'Stop'
$reviewDirectory = Join-Path ([IO.Path]::GetTempPath()) ('echo-audit-' + [guid]::NewGuid())
New-Item -ItemType Directory $reviewDirectory | Out-Null

function Install-ReviewTool($name, $url, $sha256) {
    $archive = Join-Path $reviewDirectory $name
    Invoke-WebRequest $url -OutFile $archive
    if ((Get-FileHash -LiteralPath $archive).Hash -ne $sha256) {
        throw "Digest mismatch for $name"
    }
    tar -xf $archive -C $reviewDirectory
    if ($LASTEXITCODE -ne 0) { throw "Could not extract $name" }
}

Install-ReviewTool 'audit.zip' `
    'https://github.com/rustsec/rustsec/releases/download/cargo-audit/v0.23.0/cargo-audit-x86_64-pc-windows-msvc-v0.23.0.zip' `
    'AAFEB61206730029601A72E669A12231F8674427E6248D02F6CB382CB36D56B4'
Install-ReviewTool 'deny.tar.gz' `
    'https://github.com/EmbarkStudios/cargo-deny/releases/download/0.22.0/cargo-deny-0.22.0-x86_64-pc-windows-msvc.tar.gz' `
    'A85B270B4AA28633D2CB2BE97FCCD4518CCB783014914E99463E75468CB3183C'

$audit = Get-ChildItem $reviewDirectory -Recurse -Filter cargo-audit.exe | Select-Object -First 1
$deny = Get-ChildItem $reviewDirectory -Recurse -Filter cargo-deny.exe | Select-Object -First 1
& $audit.FullName audit --file src-tauri/Cargo.lock
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& $deny.FullName --manifest-path src-tauri/Cargo.toml check licenses sources
exit $LASTEXITCODE
