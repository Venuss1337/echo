# Use the current private candidate recipe with a disposable installer identity.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [switch] $PublicRadio,
    [switch] $FixtureRadio,
    [switch] $M6,
    [ValidateRange(0, 100)][int] $ProfileCycles = 0,
    [switch] $ProfileWithoutMini,
    [guid] $FixtureToken = [guid]::NewGuid()
)
$ErrorActionPreference = 'Stop'
if ($PublicRadio -and $FixtureRadio) { throw 'Select public stations or fixed fixtures' }
$stage = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$token = $FixtureToken.ToString('N')
$work = Join-Path ([IO.Path]::GetTempPath()) "echo-installed-radio-$token"
$destination = Join-Path $work 'Installed próba 日本'
$configPath = Join-Path $work 'tauri-radio-smoke.json'
$identity = "org.echo.player.radio-install-smoke-$token"
$binary = 'echo-radio-install-smoke'
$product = "ECHO Radio Smoke $($token.Substring(0, 8))"
$manifest = Join-Path $stage 'dlls.json'
$previous = $env:ECHO_NATIVE_DLL_MANIFEST
$installer = $null
if ((Test-Path -LiteralPath $work) -or (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA $identity))) {
    throw 'The disposable install fixture already exists'
}

function Invoke-Silent($Executable, $Arguments) {
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    try {
        if (!$process.WaitForExit(120000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw 'Radio smoke installer exceeded 120 seconds'
        }
        if ($process.ExitCode -ne 0) { throw "Radio smoke installer exit code $($process.ExitCode)" }
    } finally {
        $process.Dispose()
    }
}

New-Item -ItemType Directory -Path $work | Out-Null
Push-Location (Resolve-Path "$PSScriptRoot/../..")
try {
    $config = Get-Content -LiteralPath "$stage/tauri-candidate.json" -Raw | ConvertFrom-Json -AsHashtable
    if ($config.identifier -ne 'org.echo.player.localcandidate') {
        throw 'Build the reviewed private candidate before testing its installed radio configuration'
    }
    $config.identifier = $identity
    $config.productName = $product
    $config.mainBinaryName = $binary
    $config | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $configPath
    $env:ECHO_NATIVE_DLL_MANIFEST = $manifest
    bun run tauri build --config $configPath --features native-resources -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Installed radio smoke build failed' }
    $installer = (Get-ChildItem 'src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis' -Filter "$product*_x64-setup.exe").FullName
    if (!$installer) { throw 'Expected radio smoke installer' }
    Invoke-Silent $installer "/S /D=$destination"
    & "$PSScriptRoot/test-window-chrome.ps1" -ResourceRoot $stage -InstalledExecutable "$destination/$binary.exe" -InstalledTestIdentity $identity -Radio -PublicRadio:$PublicRadio -FixtureRadio:$FixtureRadio -M6:$M6 -ProfileCycles $ProfileCycles -ProfileWithoutMini:$ProfileWithoutMini
} finally {
    $env:ECHO_NATIVE_DLL_MANIFEST = $previous
    $uninstaller = Join-Path $destination 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        Invoke-Silent $uninstaller '/S'
    }
    if ($installer -and (Test-Path -LiteralPath $installer -PathType Leaf)) {
        Remove-Item -LiteralPath $installer -Force
    }
    $resolved = (Resolve-Path -LiteralPath $work).ProviderPath
    if ($resolved -ne [IO.Path]::GetFullPath($work) -or
        (Get-Item -LiteralPath $resolved).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
        throw 'Unexpected radio smoke work directory'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
    Pop-Location
}
