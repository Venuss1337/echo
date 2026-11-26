# Opt-in acceptance of the integrated title bar with real pointer and keyboard
# input on the interactive desktop. It moves the pointer and takes the foreground
# for about a minute, so it refuses to start unless the desktop has been idle and
# nothing is full screen. Input is only sent when it provably reaches the test
# window, and nothing but that window's own pixels is captured.
# See test-window-chrome.ps1 for the checks that need no input at all.
#
# -IdleMinutes    how long the desktop must have been untouched (default 4)
# -WaitMinutes    how long to wait for that before giving up (default 0)
# -ResourceRoot   staged reviewed decoder, for a window with working playback
# -Screenshots    directory for captures of the test window
# -SnapFlyout     also hovers the maximize button and captures the area around it
#                 while the Windows 11 Snap Layouts flyout is open
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [string] $Screenshots,
    [switch] $SnapFlyout,
    [switch] $SkipBuild,
    [double] $IdleMinutes = 4,
    [double] $WaitMinutes = 0
)
$ErrorActionPreference = 'Stop'
$stage = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$repo = (Resolve-Path "$PSScriptRoot/../..").ProviderPath
$id = [guid]::NewGuid().ToString('N')
$identity = 'org.echo.player.chrome-smoke'
$work = Join-Path ([IO.Path]::GetTempPath()) "ECHO chrome smoke $id"
$database = Join-Path $env:LOCALAPPDATA $identity
if (Test-Path -LiteralPath $database) { Remove-Item -LiteralPath $database -Recurse -Force }
if ($Screenshots) { $Screenshots = (New-Item -ItemType Directory -Force -Path $Screenshots).FullName }

Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing.Common, System.Drawing.Primitives, System.Private.Windows.GdiPlus, System.Private.Windows.Core, System.Threading.Thread, System.Runtime.InteropServices, System.Collections -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
public static class Chrome {
    public struct Rect { public int L, T, R, B; public int W => R - L; public int H => B - T; }
    public struct Pt { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint type; public Mouse mi; }
    [StructLayout(LayoutKind.Sequential)] struct Mouse { public int dx, dy; public uint data, flags, time; public IntPtr extra; public long pad; }
    [StructLayout(LayoutKind.Sequential)] struct KeyInput { public uint type; public Key ki; }
    [StructLayout(LayoutKind.Sequential)] struct Key { public ushort vk, scan; public uint flags, time; public IntPtr extra; public long pad; }
    [DllImport("user32")] public static extern bool SetProcessDpiAwarenessContext(IntPtr c);
    [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32")] public static extern bool ClientToScreen(IntPtr h, ref Pt p);
    [DllImport("user32")] public static extern bool IsZoomed(IntPtr h);
    [DllImport("user32")] public static extern bool IsIconic(IntPtr h);
    [DllImport("user32")] public static extern bool ShowWindow(IntPtr h, int c);
    [DllImport("user32")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32")] public static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [DllImport("user32")] public static extern IntPtr WindowFromPoint(Pt p);
    [DllImport("user32")] public static extern IntPtr GetAncestor(IntPtr h, uint f);
    [DllImport("user32")] public static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32")] public static extern IntPtr SendMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32")] static extern uint SendInput(uint n, Input[] i, int size);
    [DllImport("user32")] static extern uint SendInput(uint n, KeyInput[] i, int size);
    [DllImport("user32")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32")] public static extern bool GetCursorPos(out Pt p);
    [DllImport("user32")] public static extern int GetSystemMetricsForDpi(int i, uint d);
    [DllImport("user32")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
    [DllImport("user32", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindowExW(IntPtr p, IntPtr a, string c, string n);
    [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint id);
    [DllImport("dwmapi")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out Rect r, int size);
    [StructLayout(LayoutKind.Sequential)] struct Last { public uint size, time; }
    [DllImport("user32")] static extern bool GetLastInputInfo(ref Last l);
    public static double IdleSeconds() {
        var l = new Last { size = 8 };
        GetLastInputInfo(ref l);
        return unchecked((uint)Environment.TickCount - l.time) / 1000.0;
    }
    /// A window that covers the whole primary screen, such as a game or a video.
    public static bool FullScreenInFront() {
        Rect r; IntPtr h = GetForegroundWindow();
        if (h == IntPtr.Zero || Class(h) == "WorkerW" || Class(h) == "Progman") return false;
        GetWindowRect(h, out r);
        return r.L <= 0 && r.T <= 0 && r.W >= GetSystemMetricsForDpi(0, 96) && r.H >= GetSystemMetricsForDpi(1, 96);
    }
    delegate bool Each(IntPtr h, IntPtr s);
    [DllImport("user32")] static extern bool EnumWindows(Each e, IntPtr s);
    [DllImport("user32")] static extern bool IsWindowVisible(IntPtr h);
    public static string Class(IntPtr h) { var s = new StringBuilder(200); GetClassNameW(h, s, 200); return s.ToString(); }
    /// The visible frame, without the invisible resize margins.
    public static Rect Frame(IntPtr h) { Rect r; DwmGetWindowAttribute(h, 9, out r, Marshal.SizeOf<Rect>()); return r; }
    public static IntPtr Point(int x, int y) => (IntPtr)(((y & 0xffff) << 16) | (x & 0xffff));
    public static int Hit(IntPtr h, int x, int y) => (int)SendMessageW(h, 0x84, IntPtr.Zero, Point(x, y));
    /// The only window this test may touch. Someone may be using this desktop:
    /// input is refused unless it would certainly reach the test window.
    public static IntPtr Own;
    static void GuardPoint(int x, int y) {
        var at = new Pt { X = x, Y = y };
        if (Own == IntPtr.Zero || GetAncestor(WindowFromPoint(at), 2) != Own)
            throw new InvalidOperationException("Refused a click that another window would receive.");
    }
    static void GuardFocus() {
        if (Own == IntPtr.Zero || GetForegroundWindow() != Own)
            throw new InvalidOperationException("Refused a key that another window would receive.");
    }
    static void Button(uint flags) {
        var i = new Input[] { new Input { type = 0, mi = new Mouse { flags = flags } } };
        SendInput(1, i, Marshal.SizeOf<Input>());
    }
    public static void Click(int x, int y) { SetCursorPos(x, y); Thread.Sleep(120); GuardPoint(x, y); Button(2); Thread.Sleep(60); Button(4); }
    public static void RightClick(int x, int y) { SetCursorPos(x, y); Thread.Sleep(120); GuardPoint(x, y); Button(8); Thread.Sleep(60); Button(16); }
    public static void Drag(int x, int y, int dx, int dy) {
        SetCursorPos(x, y); Thread.Sleep(150); GuardPoint(x, y); Button(2); Thread.Sleep(150);
        for (int n = 1; n <= 12; n++) { SetCursorPos(x + dx * n / 12, y + dy * n / 12); Thread.Sleep(22); }
        Thread.Sleep(150); Button(4);
    }
    public static void Press(params ushort[] keys) {
        int down = 0;
        try { foreach (var k in keys) { GuardFocus(); Send(k, 0); down++; Thread.Sleep(30); } }
        finally { for (int n = down - 1; n >= 0; n--) { Send(keys[n], 2); Thread.Sleep(30); } }
    }
    static void Send(ushort k, uint f) {
        var i = new KeyInput[] { new KeyInput { type = 1, ki = new Key { vk = k, flags = f } } };
        SendInput(1, i, Marshal.SizeOf<KeyInput>());
    }
    /// Visible top-level windows of one process and class.
    public static bool HasWindow(uint process, string name) {
        bool found = false;
        EnumWindows((h, s) => { uint p; GetWindowThreadProcessId(h, out p); if (p == process && IsWindowVisible(h) && Class(h) == name) { found = true; return false; } return true; }, IntPtr.Zero);
        return found;
    }
    public static Bitmap Shot(IntPtr h) {
        Rect r; GetWindowRect(h, out r); Rect f = Frame(h);
        var b = new Bitmap(r.W, r.H);
        using (var g = Graphics.FromImage(b)) { var d = g.GetHdc(); PrintWindow(h, d, 2); g.ReleaseHdc(d); }
        return b.Clone(new Rectangle(f.L - r.L, f.T - r.T, f.W, f.H), b.PixelFormat);
    }
}
'@
[Chrome]::SetProcessDpiAwarenessContext([IntPtr]-4) | Out-Null

$deadline = [DateTime]::UtcNow.AddMinutes($WaitMinutes)
# Windows delivers no input to applications behind the lock screen.
# Its process may start after the lock screen does; no window is in front then.
function Test-Locked { [bool](Get-Process -Name LogonUI -ErrorAction SilentlyContinue) -or [Chrome]::GetForegroundWindow() -eq [IntPtr]::Zero }
while ((Test-Locked) -or [Chrome]::IdleSeconds() -lt $IdleMinutes * 60 -or [Chrome]::FullScreenInFront()) {
    if ([DateTime]::UtcNow -ge $deadline) { throw 'The desktop is in use or locked. Nothing was started.' }
    Start-Sleep -Seconds 10
}

$results = [ordered]@{}
function Check([string] $name, [bool] $ok, [string] $detail = '') {
    $results[$name] = $ok
    Write-Host ("{0} - {1}{2}" -f ($(if ($ok) { 'ok' } else { 'FAILED' }), $name, $(if ($detail) { " ($detail)" })))
}
function Wait-Until([scriptblock] $test, [int] $ms = 2500) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.ElapsedMilliseconds -lt $ms) { if (& $test) { return $true }; Start-Sleep -Milliseconds 60 }
    return [bool](& $test)
}
function Save-Shot([string] $name) {
    if (!$Screenshots) { return }
    Start-Sleep -Milliseconds 500
    $image = [Chrome]::Shot($hwnd)
    $image.Save((Join-Path $Screenshots "$name.png"))
    $image.Dispose()
}
# Screen coordinates of a point in the page, given in CSS pixels from the top-right.
function At([double] $fromRight, [double] $fromTop) {
    $client = [Chrome+Rect]::new(); [Chrome]::GetClientRect($hwnd, [ref]$client) | Out-Null
    $point = [Chrome+Pt]::new(); [Chrome]::ClientToScreen($hwnd, [ref]$point) | Out-Null
    $scale = [Chrome]::GetDpiForWindow($hwnd) / 96
    return @([int]($point.X + $client.W - $fromRight * $scale), [int]($point.Y + $fromTop * $scale))
}

# Places the window above everything else and activates it with a real click, so
# that input can be proven to reach it.
function Set-Subject {
    [Chrome]::Own = $hwnd
    [Chrome]::SetWindowPos($hwnd, [IntPtr]-1, 160, 120, 1320, 840, 0x40) | Out-Null
    Start-Sleep -Milliseconds 500
    if ([Chrome]::GetForegroundWindow() -ne $hwnd) {
        $point = At 400 27
        [Chrome]::Click($point[0], $point[1])
        Start-Sleep -Milliseconds 400
    }
}
New-Item -ItemType Directory -Path $work, "$work/native" | Out-Null
$cursor = [Chrome+Pt]::new(); [Chrome]::GetCursorPos([ref]$cursor) | Out-Null
$variables = @{
    ECHO_NATIVE_DLL_MANIFEST = $env:ECHO_NATIVE_DLL_MANIFEST
    WEBVIEW2_USER_DATA_FOLDER = $env:WEBVIEW2_USER_DATA_FOLDER
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
}
Push-Location $repo
$app = $null
try {
    $env:ECHO_NATIVE_DLL_MANIFEST = Join-Path $stage 'dlls.json'
    if (!$SkipBuild) {
        $overlay = @{ identifier = $identity; productName = 'ECHO Chrome Smoke'; mainBinaryName = 'echo-chrome-smoke'; bundle = @{ active = $false } }
        $config = Join-Path $work 'tauri-chrome.json'
        $overlay | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $config
        bun run tauri build --no-bundle --debug --features native-resources --config $config -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Chrome smoke build failed' }
    }
    $binary = Join-Path $work 'echo-chrome-smoke.exe'
    Copy-Item -LiteralPath 'src-tauri/target/x86_64-pc-windows-msvc/debug/echo-chrome-smoke.exe' -Destination $binary
    foreach ($name in 'libmpv-2.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll') {
        Copy-Item -LiteralPath "$stage/native/$name" -Destination "$work/native/$name"
    }
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $work 'webview'
    $app = Start-Process -FilePath $binary -PassThru
    if (!(Wait-Until { $app.Refresh(); $app.MainWindowHandle -ne [IntPtr]::Zero } 15000)) { throw 'No window' }
    $hwnd = $app.MainWindowHandle
    if (!(Wait-Until { [Chrome]::FindWindowExW($hwnd, [IntPtr]::Zero, 'ECHO_CAPTION_TARGET', [NullString]::Value) -ne [IntPtr]::Zero } 10000)) { throw 'No hit target' }
    Start-Sleep -Seconds 3
    Set-Subject
    $dpi = [Chrome]::GetDpiForWindow($hwnd)
    Write-Host "Windows $([Environment]::OSVersion.Version) dpi=$dpi"

    # Frame ------------------------------------------------------------------
    $style = [Chrome]::GetWindowLongPtrW($hwnd, -16)
    Check 'native frame styles survive without a native caption' (($style -band 0x40000) -and ($style -band 0x10000) -and ($style -band 0x20000) -and ($style -band 0x80000)) ('style 0x{0:x}' -f $style)
    $client = [Chrome+Rect]::new(); [Chrome]::GetClientRect($hwnd, [ref]$client) | Out-Null
    $origin = [Chrome+Pt]::new(); [Chrome]::ClientToScreen($hwnd, [ref]$origin) | Out-Null
    $frame = [Chrome]::Frame($hwnd)
    Check 'the page reaches the top edge of the frame' (($origin.Y - $frame.T) -le 1) "offset $($origin.Y - $frame.T)"
    Save-Shot '01-restored'

    # Hit testing --------------------------------------------------------------
    $target = [Chrome]::FindWindowExW($hwnd, [IntPtr]::Zero, 'ECHO_CAPTION_TARGET', [NullString]::Value)
    $max = At 69 30; $min = At 115 30; $close = At 23 30
    $under = { param($p) $pt = [Chrome+Pt]::new(); $pt.X = $p[0]; $pt.Y = $p[1]; [Chrome]::WindowFromPoint($pt) }
    Check 'the maximize button is the native target' ((& $under $max) -eq $target)
    Check 'it answers HTMAXBUTTON, which Snap Layouts requires' ([Chrome]::Hit($target, $max[0], $max[1]) -eq 9)
    Check 'minimize and close stay with the page' (((& $under $min) -ne $target) -and ((& $under $close) -ne $target))
    $strip = At 69 1
    Check 'the resize strip above it still belongs to the frame' ([Chrome]::Hit($target, $strip[0], $strip[1]) -eq -1)

    # Dragging ------------------------------------------------------------------
    $drag = At 420 26
    $before = [Chrome]::Frame($hwnd)
    [Chrome]::Drag($drag[0], $drag[1], 70, 46)
    $moved = Wait-Until { $r = [Chrome]::Frame($hwnd); [Math]::Abs($r.L - $before.L - 70) -le 12 -and [Math]::Abs($r.T - $before.T - 46) -le 12 }
    $after = [Chrome]::Frame($hwnd)
    Check 'dragging the title bar moves the window' $moved "moved $($after.L - $before.L), $($after.T - $before.T)"

    # Controls inside the band are not drag regions: pressing search and moving must not.
    $before = [Chrome]::Frame($hwnd)
    $search = @(($before.L + [int](460 * $dpi / 96)), ($before.T + [int](27 * $dpi / 96)))
    [Chrome]::Drag($search[0], $search[1], 40, 30)
    Start-Sleep -Milliseconds 300
    $after = [Chrome]::Frame($hwnd)
    Check 'the search field does not drag the window' (($after.L -eq $before.L) -and ($after.T -eq $before.T))

    # Maximize ---------------------------------------------------------------------
    $drag = At 420 26
    [Chrome]::SetCursorPos($drag[0], $drag[1]) | Out-Null
    Start-Sleep -Milliseconds 150
    [Chrome]::Click($drag[0], $drag[1]); Start-Sleep -Milliseconds 40; [Chrome]::Click($drag[0], $drag[1])
    Check 'double-clicking the title bar maximizes' (Wait-Until { [Chrome]::IsZoomed($hwnd) })
    Start-Sleep -Milliseconds 600
    $max = At 69 30
    $target = [Chrome]::FindWindowExW($hwnd, [IntPtr]::Zero, 'ECHO_CAPTION_TARGET', [NullString]::Value)
    Check 'the target follows the maximized window' ((& $under $max) -eq $target)
    Add-Type -AssemblyName System.Windows.Forms
    $area = [System.Windows.Forms.Screen]::FromHandle($hwnd).WorkingArea
    $origin = [Chrome+Pt]::new(); [Chrome]::ClientToScreen($hwnd, [ref]$origin) | Out-Null
    [Chrome]::GetClientRect($hwnd, [ref]$client) | Out-Null
    Check 'maximized, the page fills exactly the work area' (($origin.X -eq $area.X) -and ($origin.Y -eq $area.Y) -and ($client.W -eq $area.Width) -and ($client.H -eq $area.Height)) "$($origin.X),$($origin.Y) $($client.W)x$($client.H) of $($area.Width)x$($area.Height)"
    Save-Shot '02-maximized'
    [Chrome]::Click($max[0], $max[1])
    Check 'the caption button restores' (Wait-Until { ![Chrome]::IsZoomed($hwnd) })
    Start-Sleep -Milliseconds 600
    $max = At 69 30
    [Chrome]::SetCursorPos($max[0], $max[1]) | Out-Null
    Start-Sleep -Milliseconds 250
    Save-Shot '03-maximize-hover'
    if ($SnapFlyout) {
        # Windows opens the flyout after a short hover on an HTMAXBUTTON hit.
        $region = { $f = [Chrome]::Frame($hwnd); New-Object Drawing.Rectangle ($f.R - 520), ($f.T), 520, 380 }
        $r = & $region
        $image = New-Object Drawing.Bitmap $r.Width, $r.Height
        $g = [Drawing.Graphics]::FromImage($image); $g.CopyFromScreen($r.X, $r.Y, 0, 0, $r.Size); $g.Dispose()
        [Chrome]::SetCursorPos($max[0] + 2, $max[1]) | Out-Null
        Start-Sleep -Milliseconds 1500
        $open = New-Object Drawing.Bitmap $r.Width, $r.Height
        $g = [Drawing.Graphics]::FromImage($open); $g.CopyFromScreen($r.X, $r.Y, 0, 0, $r.Size); $g.Dispose()
        $changed = 0; $total = 0
        for ($x = 0; $x -lt $r.Width; $x += 6) { for ($y = 64; $y -lt 330; $y += 6) { $total++; if ($image.GetPixel($x, $y) -ne $open.GetPixel($x, $y)) { $changed++ } } }
        if ($Screenshots) { $open.Save((Join-Path $Screenshots '04-snap-layouts.png')) }
        Check 'hovering maximize opens a flyout over the window' ($changed / $total -gt 0.1) ("{0:p0} of the area below the caption changed" -f ($changed / $total))
        $image.Dispose(); $open.Dispose()
        [Chrome]::SetCursorPos($max[0] - 500, $max[1] + 400) | Out-Null
        Start-Sleep -Milliseconds 700
    }
    [Chrome]::Click($max[0], $max[1])
    Check 'the caption button maximizes' (Wait-Until { [Chrome]::IsZoomed($hwnd) })
    [Chrome]::Press(0x5B, 0x28); Start-Sleep -Milliseconds 500
    Check 'Win+Down restores' (Wait-Until { ![Chrome]::IsZoomed($hwnd) })

    # Snapping ------------------------------------------------------------------------
    Set-Subject
    [Chrome]::Press(0x5B, 0x25); Start-Sleep -Milliseconds 900
    $frame = [Chrome]::Frame($hwnd)
    Check 'Win+Left snaps to half of the work area' (([Math]::Abs($frame.W - $area.Width / 2) -le 8) -and ([Math]::Abs($frame.H - $area.Height) -le 8)) "$($frame.W)x$($frame.H)"
    Save-Shot '05-snapped-half'
    # Snap Assist may hold the focus; activating the window dismisses it.
    [Chrome]::ShowWindow($hwnd, 9) | Out-Null
    Set-Subject

    # Resizing --------------------------------------------------------------------------
    foreach ($edge in @(
            @{ n = 'right edge'; x = { param($f) $f.R + 3 }; y = { param($f) [int](($f.T + $f.B) / 2) }; dx = 60; dy = 0 },
            @{ n = 'bottom-right corner'; x = { param($f) $f.R + 3 }; y = { param($f) $f.B + 3 }; dx = -50; dy = -40 },
            @{ n = 'top edge'; x = { param($f) [int](($f.L + $f.R) / 2) }; y = { param($f) $f.T + 1 }; dx = 0; dy = -30 },
            @{ n = 'left edge'; x = { param($f) $f.L - 3 }; y = { param($f) [int](($f.T + $f.B) / 2) }; dx = -40; dy = 0 })) {
        $before = [Chrome]::Frame($hwnd)
        [Chrome]::Drag((& $edge.x $before), (& $edge.y $before), $edge.dx, $edge.dy)
        $resized = Wait-Until { $f = [Chrome]::Frame($hwnd); ($f.W -ne $before.W) -or ($f.H -ne $before.H) } 1500
        $after = [Chrome]::Frame($hwnd)
        Check "resizing from the $($edge.n)" $resized "$($after.W - $before.W), $($after.H - $before.H)"
    }
    [Chrome]::SetWindowPos($hwnd, [IntPtr]-1, 160, 120, 300, 300, 0x40) | Out-Null
    Start-Sleep -Milliseconds 400
    $frame = [Chrome]::Frame($hwnd)
    Check 'the minimum size holds' (($frame.W -ge [int](420 * $dpi / 96) - 2) -and ($frame.H -ge [int](540 * $dpi / 96) - 2)) "$($frame.W)x$($frame.H)"
    Save-Shot '06-minimum'
    Set-Subject

    # System menu --------------------------------------------------------------------------
    $drag = At 420 26
    [Chrome]::RightClick($drag[0], $drag[1])
    Check 'right-clicking the title bar opens the system menu' (Wait-Until { [Chrome]::HasWindow($app.Id, '#32768') })
    [Chrome]::Press(0x1B); Start-Sleep -Milliseconds 300
    [Chrome]::Press(0x12, 0x20)
    Check 'Alt+Space opens the system menu' (Wait-Until { [Chrome]::HasWindow($app.Id, '#32768') })
    [Chrome]::Press(0x1B); Start-Sleep -Milliseconds 300

    # Minimize, taskbar restore, close ---------------------------------------------------------
    $min = At 115 30
    [Chrome]::Click($min[0], $min[1])
    Check 'the caption button minimizes' (Wait-Until { [Chrome]::IsIconic($hwnd) })
    Start-Sleep -Milliseconds 400
    [Chrome]::ShowWindow($hwnd, 9) | Out-Null
    Check 'the taskbar restore brings it back' (Wait-Until { ![Chrome]::IsIconic($hwnd) })
    Set-Subject
    [Chrome]::Press(0x12, 0x73)
    Check 'Alt+F4 closes through the ordinary shutdown' ($app.WaitForExit(9000) -and $app.ExitCode -eq 0) "exit $($app.ExitCode)"

    $app = Start-Process -FilePath $binary -PassThru
    if (!(Wait-Until { $app.Refresh(); $app.MainWindowHandle -ne [IntPtr]::Zero } 15000)) { throw 'No window on restart' }
    $hwnd = $app.MainWindowHandle
    Start-Sleep -Seconds 3
    Set-Subject
    $close = At 23 30
    [Chrome]::Click($close[0], $close[1])
    Check 'the close button closes through the ordinary shutdown' ($app.WaitForExit(9000) -and $app.ExitCode -eq 0) "exit $($app.ExitCode)"
} finally {
    [Chrome]::SetCursorPos($cursor.X, $cursor.Y) | Out-Null
    if ($app -and !$app.HasExited) { $app.Kill($true); $app.WaitForExit(5000) | Out-Null }
    Pop-Location
    foreach ($name in $variables.Keys) { [Environment]::SetEnvironmentVariable($name, $variables[$name], 'Process') }
    Start-Sleep -Milliseconds 800
    foreach ($path in $work, $database) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force -ErrorAction SilentlyContinue }
    }
}
$failed = @($results.Keys | Where-Object { !$results[$_] })
Write-Host "$($results.Count - $failed.Count) of $($results.Count) title bar checks passed"
if ($failed.Count) { throw "Failed: $($failed -join '; ')" }
