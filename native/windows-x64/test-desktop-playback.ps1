# Opt-in Windows desktop acceptance. Plays original audio with the ECHO session muted.
# -Journey additionally drives the folder picker, library, playlists, favorites,
# history, Now Playing and restart through the production application.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [switch] $Journey,
    # Optional directory for screenshots of the real WebView2 window.
    [string] $Screenshots
)
$ErrorActionPreference = 'Stop'
$stage = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$repo = (Resolve-Path "$PSScriptRoot/../..").ProviderPath
$temporary = [IO.Path]::GetTempPath()
$id = [guid]::NewGuid().ToString('N')
$identity = "org.echo.player.playback-smoke-$id"
$work = Join-Path $temporary "ECHO playback smoke 音 $id"
$database = Join-Path $env:LOCALAPPDATA $identity
if ((Test-Path -LiteralPath $work) -or (Test-Path -LiteralPath $database)) {
    throw 'Expected fresh desktop smoke directories'
}
$binary = Join-Path $work 'echo-playback-smoke.exe'
$fixture = Join-Path $work 'ECHO selection 音.wav'

# Select the fixture only in the visible, owned dialog of the exact test process.
if (!('EchoOwnedFileDialog' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class EchoOwnedFileDialog {
    delegate bool Enumerator(IntPtr hwnd, IntPtr state);
    [DllImport("user32")] static extern bool EnumWindows(Enumerator callback, IntPtr state);
    [DllImport("user32")] static extern bool EnumChildWindows(IntPtr hwnd, Enumerator callback, IntPtr state);
    [DllImport("user32")] static extern IntPtr GetParent(IntPtr hwnd);
    [DllImport("user32")] static extern int GetDlgCtrlID(IntPtr hwnd);
    [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint id);
    [DllImport("user32")] static extern IntPtr GetWindow(IntPtr hwnd, uint flag);
    [DllImport("user32", CharSet=CharSet.Unicode)] static extern int GetClassNameW(IntPtr hwnd, StringBuilder name, int count);
    [DllImport("user32")] static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32")] public static extern bool PostMessageW(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
    [DllImport("user32", CharSet=CharSet.Unicode)] static extern IntPtr SendMessageTimeoutW(IntPtr hwnd, uint msg, IntPtr w, string text, uint flags, uint timeout, out IntPtr result);
    static string Class(IntPtr hwnd) {
        var name = new StringBuilder(120);
        GetClassNameW(hwnd, name, name.Capacity);
        return name.ToString();
    }
    public static IntPtr Find(uint process) {
        IntPtr result = IntPtr.Zero;
        EnumWindows((window, state) => {
            uint id, ownerId;
            GetWindowThreadProcessId(window, out id);
            IntPtr owner = GetWindow(window, 4); // GW_OWNER
            if (id != process || owner == IntPtr.Zero || !IsWindowVisible(window) || Class(window) != "#32770") return true;
            GetWindowThreadProcessId(owner, out ownerId);
            if (ownerId != process) return true;
            result = window;
            return false;
        }, IntPtr.Zero);
        return result;
    }
    public static bool Choose(IntPtr dialog, string path) {
        IntPtr edit = IntPtr.Zero, button = IntPtr.Zero;
        EnumChildWindows(dialog, (window, state) => {
            if (!IsWindowVisible(window)) return true;
            string kind = Class(window);
            if (kind == "Button" && GetDlgCtrlID(window) == 1) button = window;
            if (kind == "Edit" && Class(GetParent(window)).StartsWith("ComboBox")) edit = window;
            // The folder picker names its field directly (control id 1152).
            if (kind == "Edit" && edit == IntPtr.Zero && GetDlgCtrlID(window) == 1152) edit = window;
            return true;
        }, IntPtr.Zero);
        if (edit == IntPtr.Zero || button == IntPtr.Zero) return false;
        IntPtr result;
        const uint WM_SETTEXT = 12, BM_CLICK = 0xf5, SMTO_ABORTIFHUNG = 2;
        return SendMessageTimeoutW(edit, WM_SETTEXT, IntPtr.Zero, path, SMTO_ABORTIFHUNG, 2000, out result) != IntPtr.Zero &&
               PostMessageW(button, BM_CLICK, IntPtr.Zero, IntPtr.Zero);
    }
}
'@
}

function Test-DesktopPhase([string] $phase, [string] $script, [string] $choose, [int] $limit) {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start()
    $port = $listener.LocalEndpoint.Port
    $listener.Stop()
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-address=127.0.0.1 --remote-debugging-port=$port"
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $work "webview-$phase"
    $app = $null
    $client = $null
    $dialog = [IntPtr]::Zero
    try {
        $app = Start-Process -FilePath $binary -WindowStyle Hidden -PassThru
        $clock = [Diagnostics.Stopwatch]::StartNew()
        $ready = $null
        do {
            if ($app.HasExited) { throw "Desktop process exited: $($app.ExitCode)" }
            try {
                $ready = Invoke-RestMethod "http://127.0.0.1:$port/json/version" -NoProxy -TimeoutSec 1 -ErrorAction Stop
            } catch {
                Start-Sleep -Milliseconds 200
            }
        } while (!$ready -and $clock.Elapsed.TotalSeconds -lt 15)
        if (!$ready) { throw 'No isolated WebView2 endpoint' }
        $client = Start-Process -FilePath (Get-Command bun).Source -ArgumentList (@($script, $port, $phase) + $(if ($Screenshots) { "`"$Screenshots`"" })) -WorkingDirectory $repo -RedirectStandardOutput "$work/$phase-out.txt" -RedirectStandardError "$work/$phase-error.txt" -WindowStyle Hidden -PassThru
        if ($choose) {
            $clock.Restart()
            do {
                $dialog = [EchoOwnedFileDialog]::Find([uint32]$app.Id)
                if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
            } while ($dialog -eq [IntPtr]::Zero -and !$client.HasExited -and $clock.Elapsed.TotalSeconds -lt 20)
            if ($dialog -eq [IntPtr]::Zero) { throw 'The owned picker did not open' }
            # The dialog creates its controls after it becomes visible.
            $clock.Restart()
            while (![EchoOwnedFileDialog]::Choose($dialog, $choose)) {
                if ($clock.Elapsed.TotalSeconds -gt 8) { throw 'Could not select the generated fixture through the owned picker' }
                Start-Sleep -Milliseconds 150
            }
            # A folder dialog may treat the first click as navigation into it.
            $clock.Restart()
            while ([EchoOwnedFileDialog]::Find([uint32]$app.Id) -eq $dialog -and $clock.Elapsed.TotalSeconds -lt 6) {
                Start-Sleep -Milliseconds 700
                if ([EchoOwnedFileDialog]::Find([uint32]$app.Id) -eq $dialog) {
                    [EchoOwnedFileDialog]::Choose($dialog, $choose) | Out-Null
                }
            }
        }
        if (!$client.WaitForExit($limit)) {
            Get-Content -LiteralPath "$work/$phase-out.txt", "$work/$phase-error.txt"
            throw "Desktop $phase check timed out"
        }
        Get-Content -LiteralPath "$work/$phase-out.txt", "$work/$phase-error.txt"
        if ($client.ExitCode -ne 0) { throw "Desktop $phase check exited $($client.ExitCode)" }
    } finally {
        if ($dialog -ne [IntPtr]::Zero -and $app -and [EchoOwnedFileDialog]::Find([uint32]$app.Id) -eq $dialog) {
            [EchoOwnedFileDialog]::PostMessageW($dialog, 16, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        }
        foreach ($process in @($client, $app)) {
            if (!$process) { continue }
            try {
                $process.Refresh()
                if (!$process.HasExited) {
                    $null = $process.CloseMainWindow()
                    if (!$process.WaitForExit(8000)) {
                        $process.Kill($true)
                        $process.WaitForExit(8000) | Out-Null
                    }
                }
                if ($process -eq $app -and $process.ExitCode -ne 0) {
                    throw "Desktop exit code $($process.ExitCode)"
                }
            } finally {
                $process.Dispose()
            }
        }
    }
}

function Remove-TestDirectory([string] $path, [string] $expectedParent) {
    if (!(Test-Path -LiteralPath $path -PathType Container)) { return }
    $actual = (Resolve-Path -LiteralPath $path).ProviderPath
    $parent = [IO.Path]::TrimEndingDirectorySeparator((Resolve-Path -LiteralPath $expectedParent).ProviderPath)
    if ($actual -ne [IO.Path]::GetFullPath($path) -or
        [IO.Path]::GetDirectoryName($actual) -ne $parent -or
        (Get-Item -LiteralPath $actual).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
        throw "Unexpected desktop smoke directory: $path"
    }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while (Test-Path -LiteralPath $actual) {
        try {
            Remove-Item -LiteralPath $actual -Recurse -Force -ErrorAction Stop
        } catch {
            if ($clock.Elapsed.TotalSeconds -ge 10) { throw }
            Start-Sleep -Milliseconds 200
        }
    }
}

$variables = @{
    ECHO_NATIVE_DLL_MANIFEST = $env:ECHO_NATIVE_DLL_MANIFEST
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
    WEBVIEW2_USER_DATA_FOLDER = $env:WEBVIEW2_USER_DATA_FOLDER
}
New-Item -ItemType Directory -Path $work, "$work/native" | Out-Null
Push-Location $repo
$passed = $false
try {
    $env:ECHO_NATIVE_DLL_MANIFEST = Join-Path $stage 'dlls.json'
    $overlay = @{ identifier = $identity; productName = 'ECHO Playback Smoke'; mainBinaryName = 'echo-playback-smoke'; bundle = @{ active = $false } }
    $config = Join-Path $work 'tauri-smoke.json'
    $overlay | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $config
    bun run tauri build --no-bundle --debug --features native-resources --config $config -- --locked
    if ($LASTEXITCODE -ne 0) { throw 'Desktop smoke build failed' }
    bun run fixtures:audio
    if ($LASTEXITCODE -ne 0) { throw 'Audio fixture generation failed' }
    Copy-Item -LiteralPath '.echo-fixtures/transport.wav' -Destination $fixture
    Copy-Item -LiteralPath 'src-tauri/target/x86_64-pc-windows-msvc/debug/echo-playback-smoke.exe' -Destination $binary
    foreach ($name in 'libmpv-2.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll') {
        Copy-Item -LiteralPath "$stage/native/$name" -Destination "$work/native/$name"
    }
    if ($Journey) {
        if ($Screenshots) { $Screenshots = (New-Item -ItemType Directory -Force -Path $Screenshots).FullName }
        $music = Join-Path $work 'Muzyka próba 音'
        cargo run --locked --quiet --manifest-path src-tauri/Cargo.toml --example journey-fixtures -- $music
        if ($LASTEXITCODE -ne 0) { throw 'Music fixture generation failed' }
        $before = Get-ChildItem -LiteralPath $music -Recurse -File | Get-FileHash
        Test-DesktopPhase journey 'scripts/test-desktop-journey.ts' $music 120000
        Test-DesktopPhase journey-restart 'scripts/test-desktop-journey.ts' '' 60000
        $after = Get-ChildItem -LiteralPath $music -Recurse -File | Get-FileHash
        if (Compare-Object $before $after -Property Path, Hash) { throw 'The journey changed a music file' }
        Write-Host 'Music files are byte-identical after the journey.'
    } else {
        Test-DesktopPhase play 'scripts/test-desktop-playback.ts' $fixture 24000
        Test-DesktopPhase restart 'scripts/test-desktop-playback.ts' '' 24000
    }
    $passed = $true
} finally {
    Pop-Location
    foreach ($name in $variables.Keys) {
        [Environment]::SetEnvironmentVariable($name, $variables[$name], 'Process')
    }
    if ($passed) {
        Remove-TestDirectory $work $temporary
        Remove-TestDirectory $database $env:LOCALAPPDATA
    } else {
        # Both fresh paths contain only generated test data.
        Write-Host "Desktop smoke files: $work"
        Write-Host "Desktop smoke data: $database"
    }
}
