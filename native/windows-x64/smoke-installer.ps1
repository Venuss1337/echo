# Installs a private candidate or a release installer to an isolated location,
# verifies its decoder and notices, runs the headless suite against the
# installed DLLs and uninstalls it.
param(
    [Parameter(Mandatory)][string] $Installer,
    [Parameter(Mandatory)][string] $Manifest,
    [string] $Ffmpeg,
    [switch] $VerifyWebView,
    [switch] $VerifyRadio,
    [switch] $PublicRadio
)
$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path -LiteralPath $Installer).ProviderPath
$Manifest = (Resolve-Path -LiteralPath $Manifest).ProviderPath
$leaf = Split-Path $Installer -Leaf
# Release installers ship the reviewed local decoder under the production
# identity; their decoder notices live under notices/native.
$release = $leaf -cmatch '^(ECHO_\d+\.\d+\.\d+_x64|ECHO-\d+\.\d+\.\d+-windows-x64)-setup\.exe$'
if (!$release -and $leaf -notmatch '^ECHO (Native|Local) Candidate_.+_x64-setup\.exe$') {
    throw 'Expected an ECHO Windows x64 installer'
}
$local = $release -or $Matches[1] -eq 'Local'
if ($VerifyRadio -and $release) { throw 'Radio smoke runs against the private local candidate' }
if (($VerifyRadio -and !$local) -or ($PublicRadio -and !$VerifyRadio)) { throw 'Radio smoke requires a local candidate and -VerifyRadio' }
$executable = if ($release) { 'echo.exe' } elseif ($local) { 'echo-local-candidate.exe' } else { 'echo-native-candidate.exe' }
$installedNotices = if ($release) { 'notices/native' } else { 'notices' }
if ($release -and (
    (Test-Path -LiteralPath (Join-Path $env:LOCALAPPDATA 'org.echo.player')) -or
    (Test-Path -LiteralPath 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/ECHO') -or
    (Test-Path -LiteralPath 'HKLM:/Software/Microsoft/Windows/CurrentVersion/Uninstall/ECHO') -or
    (Test-Path -LiteralPath 'HKLM:/Software/WOW6432Node/Microsoft/Windows/CurrentVersion/Uninstall/ECHO')
)) {
    throw 'The production installer smoke test requires a Windows account with no existing ECHO data or registration.'
}
$destination = Join-Path ([IO.Path]::GetTempPath()) ("ECHO Native próba 日本 " + [guid]::NewGuid())
if (Test-Path -LiteralPath $destination) { throw 'Smoke test destination must be fresh' }

function Invoke-Silent($Executable, $Arguments) {
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    try {
        if (!$process.WaitForExit(120000)) {
            $process.Kill($true)
            $process.WaitForExit()
            throw 'Smoke process exceeded 120 seconds'
        }
        if ($process.ExitCode -ne 0) { throw "Smoke process exit code $($process.ExitCode)" }
    } finally {
        $process.Dispose()
    }
}

function Test-LocalWebView($Executable) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    $priorFlags = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    $priorData = $env:WEBVIEW2_USER_DATA_FOLDER
    $process = $null
    try {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-address=127.0.0.1 --remote-debugging-port=$port"
        $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $destination 'webview-smoke'
        $process = Start-Process -FilePath $Executable -PassThru -WindowStyle Hidden
        $deadline = [Diagnostics.Stopwatch]::StartNew()
        do {
            $process.Refresh()
            if ($process.HasExited) { throw "WebView2 candidate exited: $($process.ExitCode)" }
            try {
                $ready = Invoke-RestMethod "http://127.0.0.1:$port/json/version" -TimeoutSec 1 -NoProxy -ErrorAction Stop
            } catch {
                Start-Sleep -Milliseconds 180
            }
        } while (!$ready -and $deadline.Elapsed.TotalSeconds -lt 15)
        if (!$ready) { throw 'No isolated WebView2 debugging endpoint' }
        Push-Location (Resolve-Path "$PSScriptRoot/../..")
        try {
            bun scripts/test-desktop-ipc.ts $port
            if ($LASTEXITCODE -ne 0) { throw 'Installed WebView2 IPC smoke failed' }
        } finally {
            Pop-Location
        }
    } finally {
        $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $priorFlags
        $env:WEBVIEW2_USER_DATA_FOLDER = $priorData
        if ($process) {
            try {
                $process.Refresh()
                if (!$process.HasExited) {
                    $null = $process.CloseMainWindow()
                    if (!$process.WaitForExit(10000)) {
                        $process.Kill($true)
                        $process.WaitForExit(10000) | Out-Null
                        throw 'Installed WebView2 process did not exit cleanly'
                    }
                }
                if ($process.ExitCode -ne 0) { throw "WebView2 exited with $($process.ExitCode)" }
            } finally {
                $process.Dispose()
            }
        }
        $profile = Join-Path $destination 'webview-smoke'
        if (Test-Path -LiteralPath $profile) {
            $resolved = (Resolve-Path -LiteralPath $profile).ProviderPath
            if ($resolved -ne [IO.Path]::GetFullPath($profile) -or
                (Get-Item -LiteralPath $resolved).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
                throw 'Unexpected WebView2 smoke profile'
            }
            # WebView2 helper processes can finish shortly after their owner.
            $cleanup = [Diagnostics.Stopwatch]::StartNew()
            while (Test-Path -LiteralPath $resolved) {
                try {
                    Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction Stop
                } catch {
                    if ($cleanup.Elapsed.TotalSeconds -ge 10) { throw }
                    Start-Sleep -Milliseconds 200
                }
            }
        }
    }
}

try {
    # NSIS interprets everything after /D= as the directory, including spaces.
    Invoke-Silent $Installer "/S /D=$destination"
    if (!(Test-Path -LiteralPath "$destination/uninstall.exe" -PathType Leaf)) {
        throw 'Expected candidate uninstaller missing'
    }
    Invoke-Silent "$destination/$executable" '--verify-native-resources'
    $notices = Join-Path (Split-Path $Manifest -Parent) 'notices'
    foreach ($file in (Get-ChildItem -LiteralPath $notices -File -Recurse)) {
        $relative = [IO.Path]::GetRelativePath($notices, $file.FullName)
        $installed = Join-Path "$destination/$installedNotices" $relative
        if ((Get-FileHash -LiteralPath $installed).Hash -ne (Get-FileHash -LiteralPath $file.FullName).Hash) {
            throw "Installed candidate notice digest mismatch: $relative"
        }
    }
    if ($release) {
        foreach ($required in 'LICENSE.txt', 'SOURCE.txt', 'notices/THIRD_PARTY_NOTICES.txt') {
            if (!(Test-Path -LiteralPath "$destination/$required" -PathType Leaf)) { throw "Release installer omits $required" }
        }
    }
    if ($local) {
        $params = @{ ResourceRoot = $destination; Manifest = $Manifest }
        if ($Ffmpeg) { $params.Ffmpeg = $Ffmpeg }
        & "$PSScriptRoot/test-local.ps1" @params
        if ($VerifyRadio) {
            & "$PSScriptRoot/test-window-chrome.ps1" -ResourceRoot (Split-Path $Manifest -Parent) -InstalledExecutable "$destination/$executable" -Radio -PublicRadio:$PublicRadio
        }
        if ($VerifyWebView) {
            if ($release) {
                & "$PSScriptRoot/test-window-chrome.ps1" -ResourceRoot (Split-Path $Manifest -Parent) -InstalledExecutable "$destination/$executable" -InstalledRelease
            } else {
                Test-LocalWebView "$destination/$executable"
            }
        }
    } else {
        & "$PSScriptRoot/test-installed.ps1" -ResourceRoot $destination -Manifest $Manifest
    }
} finally {
    $uninstaller = Join-Path $destination 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        Invoke-Silent $uninstaller '/S'
    }
}
