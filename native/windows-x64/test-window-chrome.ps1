# Acceptance of the integrated title bar on a real window in real WebView2,
# without using the interactive desktop: the application runs on a private Windows
# desktop, so nothing appears, no pointer moves and no focus changes.
#
# It covers the frame, hit testing, maximize, restore, minimize, minimum size,
# system commands, closing and the IPC boundary. Physical dragging, edge resizing,
# the Snap Layouts flyout and the system menus need real input and are in
# test-window-input.ps1.
param(
    [Parameter(Mandatory)][string] $ResourceRoot,
    [string] $Screenshots,
    [switch] $SkipBuild,
    [string] $InstalledExecutable,
    [switch] $InstalledRelease,
    [ValidatePattern('^org\.echo\.player\.radio-install-smoke-[a-f0-9]{32}$')]
    [string] $InstalledTestIdentity,
    [switch] $Radio,
    [switch] $PublicRadio,
    [switch] $FixtureRadio,
    [switch] $M6,
    [ValidateRange(0, 100)][int] $ProfileCycles = 0,
    [switch] $ProfileWithoutMini,
    # Collect garbage before every page sample, to locate retained growth.
    [switch] $RetainedSamples,
    [guid] $FixtureProfileToken = [guid]::Empty
)
$ErrorActionPreference = 'Stop'
$stage = (Resolve-Path -LiteralPath $ResourceRoot).ProviderPath
$repo = (Resolve-Path "$PSScriptRoot/../..").ProviderPath
$id = [guid]::NewGuid().ToString('N')
$identity = 'org.echo.player.chrome-smoke'
if ($InstalledRelease -and (!$InstalledExecutable -or $InstalledTestIdentity -or $FixtureProfileToken -ne [guid]::Empty)) {
    throw 'Installed release testing requires the production executable and a fresh profile'
}
if ($InstalledExecutable) {
    $InstalledExecutable = (Resolve-Path -LiteralPath $InstalledExecutable).ProviderPath
    if ($InstalledRelease) {
        if ((Split-Path $InstalledExecutable -Leaf) -cne 'echo.exe') { throw 'Expected installed echo.exe' }
        $identity = 'org.echo.player'
    } elseif ($InstalledTestIdentity) {
        if (!$Radio -or (Split-Path $InstalledExecutable -Leaf) -ne 'echo-radio-install-smoke.exe') {
            throw 'Expected the installed radio smoke build'
        }
        $identity = $InstalledTestIdentity
    } else {
        if ((Split-Path $InstalledExecutable -Leaf) -ne 'echo-local-candidate.exe') { throw 'Expected an installed local candidate' }
        $identity = 'org.echo.player.localcandidate'
    }
} elseif ($Radio -or $M6) {
    $identity = "org.echo.player.runtime-smoke-$id"
}
if (($PublicRadio -or $FixtureRadio) -and !$Radio) { throw 'Radio playback acceptance requires -Radio' }
if ($PublicRadio -and $FixtureRadio) { throw 'Select public stations or fixed fixtures' }
if ($ProfileCycles -and (!$M6 -or !$FixtureRadio)) { throw 'Profile cycles require -M6 and -FixtureRadio' }
if ($ProfileWithoutMini -and !$ProfileCycles) { throw 'ProfileWithoutMini requires ProfileCycles' }
$radioMode = if ($PublicRadio) { 'public' } elseif ($FixtureRadio) { 'fixture' } else { 'offline' }
if (($Radio -or $M6) -and $SkipBuild -and !$InstalledExecutable) { throw 'Build the isolated runtime test profile' }
$work = Join-Path ([IO.Path]::GetTempPath()) "ECHO chrome smoke $id"
$database = Join-Path $env:LOCALAPPDATA $identity
$reuseFixture = $FixtureProfileToken -ne [guid]::Empty
if ($reuseFixture) {
    $marker = Join-Path $database 'upgrade-fixture-token'
    if (!$InstalledExecutable -or !$FixtureRadio -or
        !(Test-Path -LiteralPath $marker -PathType Leaf) -or
        (Get-Content -LiteralPath $marker -Raw).Trim() -cne $FixtureProfileToken.ToString('N')) {
        throw 'Only a seeded installed upgrade fixture may reuse an existing profile'
    }
} elseif (Test-Path -LiteralPath $database) { throw 'The desktop smoke profile already exists' }
if ($Screenshots) { $Screenshots = (New-Item -ItemType Directory -Force -Path $Screenshots).FullName }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Concurrent;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
/// A private desktop and one thread that lives on it. Window messages only cross
/// between windows of the same desktop, so every call is made from that thread.
public static class Desk {
    public struct Rect { public int L, T, R, B; public int W => R - L; public int H => B - T; }
    public struct Pt { public int X, Y; }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct Startup {
        public int cb; public string reserved, desktop, title;
        public int x, y, w, h, cx, cy, fill, flags; public short show, n2; public IntPtr r2, a, b, c;
    }
    [StructLayout(LayoutKind.Sequential)] struct Process { public IntPtr process, thread; public int pid, tid; }
    [StructLayout(LayoutKind.Sequential)] struct MonitorInfo { public int cb; public Rect monitor, work; public uint flags; }
    [DllImport("user32", SetLastError = true, CharSet = CharSet.Unicode)] static extern IntPtr CreateDesktopW(string n, IntPtr d, IntPtr m, uint f, uint a, IntPtr s);
    [DllImport("user32", SetLastError = true)] static extern bool SetThreadDesktop(IntPtr d);
    [DllImport("user32")] static extern bool CloseDesktop(IntPtr d);
    [DllImport("user32")] static extern bool SetProcessDpiAwarenessContext(IntPtr c);
    [DllImport("kernel32", SetLastError = true, CharSet = CharSet.Unicode)] static extern bool CreateProcessW(string a, StringBuilder c, IntPtr p, IntPtr t, bool i, uint f, IntPtr e, string d, ref Startup s, out Process o);
    [DllImport("kernel32")] static extern bool CloseHandle(IntPtr handle);
    delegate bool Each(IntPtr h, IntPtr s);
    [DllImport("user32")] static extern bool EnumDesktopWindows(IntPtr d, Each e, IntPtr s);
    [DllImport("user32")] static extern bool EnumChildWindows(IntPtr h, Each e, IntPtr s);
    [DllImport("user32")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint id);
    [DllImport("user32", CharSet = CharSet.Unicode)] static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32")] static extern bool IsZoomed(IntPtr h);
    [DllImport("user32")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32")] static extern bool ShowWindow(IntPtr h, int command);
    [DllImport("user32")] static extern IntPtr GetWindow(IntPtr h, int command);
    [DllImport("user32", CharSet = CharSet.Unicode)] static extern IntPtr CreateWindowExW(uint ex, string cls, string title, uint style, int x, int y, int w, int h, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr param);
    [DllImport("user32")] static extern bool DestroyWindow(IntPtr h);
    [DllImport("user32")] static extern bool GetWindowRect(IntPtr h, out Rect r);
    [DllImport("user32")] static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32")] static extern bool ClientToScreen(IntPtr h, ref Pt p);
    [DllImport("user32")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32")] static extern long GetWindowLongPtrW(IntPtr h, int i);
    [DllImport("user32")] static extern bool SetWindowPos(IntPtr h, IntPtr a, int x, int y, int w, int ht, uint f);
    [DllImport("user32")] static extern IntPtr WindowFromPoint(Pt p);
    [DllImport("user32")] static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
    [DllImport("user32")] static extern bool GetMonitorInfoW(IntPtr m, ref MonitorInfo i);
    [DllImport("user32")] static extern IntPtr SendMessageTimeoutW(IntPtr h, uint m, IntPtr w, IntPtr l, uint f, uint t, out IntPtr r);
    [DllImport("user32")] static extern bool PostMessageW(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32")] static extern int GetSystemMetricsForDpi(int i, uint d);
    [DllImport("dwmapi")] static extern int DwmGetWindowAttribute(IntPtr h, int a, out Rect r, int s);
    static readonly BlockingCollection<Action> work = new BlockingCollection<Action>();
    static IntPtr desktop;
    static string name;
    public static void Open(string desktopName) {
        SetProcessDpiAwarenessContext((IntPtr)(-4));
        name = desktopName;
        desktop = CreateDesktopW(name, IntPtr.Zero, IntPtr.Zero, 0, 0x10000000, IntPtr.Zero);
        if (desktop == IntPtr.Zero) throw new Win32Exception();
        var thread = new Thread(() => {
            if (!SetThreadDesktop(desktop)) return;
            foreach (var action in work.GetConsumingEnumerable()) action();
        });
        thread.IsBackground = true;
        thread.Start();
    }
    public static void Close() { work.CompleteAdding(); CloseDesktop(desktop); }
    static T Run<T>(Func<T> body) {
        T result = default(T); Exception failure = null;
        // Not disposed: a call that outlives the wait still signals it.
        var done = new ManualResetEventSlim();
        work.Add(() => { try { result = body(); } catch (Exception e) { failure = e; } finally { done.Set(); } });
        if (!done.Wait(10000)) throw new TimeoutException("The private desktop thread did not answer.");
        if (failure != null) throw failure;
        return result;
    }
    /// Starts a process on the private desktop and returns its id.
    public static int Start(string path, string directory) {
        var s = new Startup { desktop = "WinSta0\\" + name };
        s.cb = Marshal.SizeOf<Startup>();
        Process p;
        if (!CreateProcessW(null, new StringBuilder("\"" + path + "\""), IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, directory, ref s, out p)) throw new Win32Exception();
        CloseHandle(p.thread);
        CloseHandle(p.process);
        return p.pid;
    }
    static string Class(IntPtr h) { var s = new StringBuilder(200); GetClassNameW(h, s, 200); return s.ToString(); }
    public static string ClassOf(IntPtr h) => Run(() => Class(h));
    public static IntPtr Main(int pid) => Run(() => {
        IntPtr found = IntPtr.Zero;
        EnumDesktopWindows(desktop, (h, s) => { uint p; GetWindowThreadProcessId(h, out p); if (p == pid && IsWindowVisible(h) && Class(h) == "Tauri Window") { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    });
    public static IntPtr Child(IntPtr parent, string type) => Run(() => {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (h, s) => { if (Class(h) == type) { found = h; return false; } return true; }, IntPtr.Zero);
        return found;
    });
    public static bool Zoomed(IntPtr h) => Run(() => IsZoomed(h));
    public static bool Iconic(IntPtr h) => Run(() => IsIconic(h));
    public static bool Visible(IntPtr h) => Run(() => IsWindowVisible(h));
    public static uint Dpi(IntPtr h) => Run(() => GetDpiForWindow(h));
    public static long Style(IntPtr h) => Run(() => GetWindowLongPtrW(h, -16));
    public static bool Topmost(IntPtr h) => Run(() => (GetWindowLongPtrW(h, -20) & 8) != 0);
    public static void Hide(IntPtr h) => Run(() => ShowWindow(h, 0));
    public static void Reveal(IntPtr h) => Run(() => ShowWindow(h, 8));
    // The tray's Hide to tray minimizes before hiding; Show ECHO restores.
    public static void HideToTray(IntPtr h) => Run(() => { ShowWindow(h, 6); return ShowWindow(h, 0); });
    public static void ShowFromTray(IntPtr h) => Run(() => ShowWindow(h, 9));
    public static bool AboveNormalPeer(IntPtr h) => Run(() => {
        var peer = CreateWindowExW(0x08000000, "STATIC", "ECHO acceptance peer", 0x90000000,
            32, 32, 120, 120, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        if (peer == IntPtr.Zero) throw new Win32Exception();
        try {
            if (!SetWindowPos(peer, IntPtr.Zero, 0, 0, 0, 0, 0x13)) throw new Win32Exception();
            if ((GetWindowLongPtrW(peer, -20) & 8) != 0) return false;
            for (var current = GetWindow(peer, 3); current != IntPtr.Zero; current = GetWindow(current, 3)) {
                if (current == h) return true;
            }
            return false;
        } finally { DestroyWindow(peer); }
    });
    public static Rect Window(IntPtr h) => Run(() => { Rect r; GetWindowRect(h, out r); return r; });
    public static Rect Work(IntPtr h) => Run(() => { var i = new MonitorInfo(); i.cb = Marshal.SizeOf<MonitorInfo>(); GetMonitorInfoW(MonitorFromWindow(h, 2), ref i); return i.work; });
    public static Rect Client(IntPtr h) => Run(() => {
        Rect r; GetClientRect(h, out r); var p = new Pt(); ClientToScreen(h, ref p);
        return new Rect { L = p.X, T = p.Y, R = p.X + r.R, B = p.Y + r.B };
    });
    public static void Place(IntPtr h, int x, int y, int w, int ht) => Run(() => SetWindowPos(h, IntPtr.Zero, x, y, w, ht, 0x14));
    public static int Metric(int index, uint dpi) => Run(() => GetSystemMetricsForDpi(index, dpi));
    [StructLayout(LayoutKind.Sequential)] struct MinMax { public Pt reserved, maxSize, maxPosition, minTrack, maxTrack; }
    [DllImport("user32", EntryPoint = "SendMessageW")] static extern IntPtr SendMinMax(IntPtr h, uint m, IntPtr w, ref MinMax l);
    /// The smallest outer size Windows will let a resize reach.
    public static Pt MinTrack(IntPtr h) => Run(() => { var m = new MinMax(); SendMinMax(h, 0x24, IntPtr.Zero, ref m); return m.minTrack; });
    static IntPtr Point(int x, int y) => (IntPtr)(((y & 0xffff) << 16) | (x & 0xffff));
    /// The window a pointer at this position would reach.
    public static IntPtr At(int x, int y) => Run(() => WindowFromPoint(new Pt { X = x, Y = y }));
    public static int Hit(IntPtr h, int x, int y) => (int)Send(h, 0x84, 0, Point(x, y).ToInt64());
    public static long Send(IntPtr h, uint message, long w, long l) => Run(() => {
        IntPtr r; SendMessageTimeoutW(h, message, (IntPtr)w, (IntPtr)l, 2, 4000, out r); return r.ToInt64();
    });
    public static void Post(IntPtr h, uint message, long w, long l) => Run(() => PostMessageW(h, message, (IntPtr)w, (IntPtr)l));
    public static void PostMouse(IntPtr h, uint message, int code, int x, int y) => Post(h, message, code, Point(x, y).ToInt64());
    public static void Mouse(IntPtr h, uint message, int code, int x, int y) => Send(h, message, code, Point(x, y).ToInt64());
}
'@

$results = [ordered]@{}
function Check([string] $name, [bool] $ok, [string] $detail = '') {
    $results[$name] = $ok
    Write-Host ("{0} - {1}{2}" -f ($(if ($ok) { 'ok' } else { 'FAILED' }), $name, $(if ($detail) { " ($detail)" })))
}
function Wait-Until([scriptblock] $test, [int] $ms = 4000) {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    while ($clock.ElapsedMilliseconds -lt $ms) { if (& $test) { return $true }; Start-Sleep -Milliseconds 60 }
    return [bool](& $test)
}
function Invoke-Page([string] $step, [string] $argument = '') {
    if ($script:profileDriver) {
        $script:profileDriver.StandardInput.WriteLine((@{ step = $step; argument = $argument } | ConvertTo-Json -Compress))
        $script:profileDriver.StandardInput.Flush()
        $read = $script:profileDriver.StandardOutput.ReadLineAsync()
        if (!$read.Wait(30000) -or !$read.Result) { throw "Profile page step $step failed or timed out" }
        return ($read.Result | ConvertFrom-Json)
    }
    $output = bun scripts/test-window-chrome.ts $port $step $argument
    if ($LASTEXITCODE -ne 0) { throw "Page step $step failed" }
    $output | Select-Object -Last 1 | ConvertFrom-Json
}
function Stop-ProfileDriver {
    if (!$script:profileDriver) { return }
    try {
        $script:profileDriver.StandardInput.Close()
        if (!$script:profileDriver.WaitForExit(5000)) { $script:profileDriver.Kill($true) }
    } finally {
        $script:profileDriver.Dispose()
        $script:profileDriver = $null
    }
}
function Start-Subject([switch] $Concurrent) {
    $elapsed = [Diagnostics.Stopwatch]::StartNew()
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    $listener.Start(); $script:port = $listener.LocalEndpoint.Port; $listener.Stop()
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-address=127.0.0.1 --remote-debugging-port=$port"
    $ids = @([Desk]::Start($binary, $work))
    if ($Concurrent) { $ids += [Desk]::Start($binary, $work) }
    $participants = @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
    $script:started += $participants
    if (!(Wait-Until {
        $script:app = $participants | Where-Object {
            !$_.HasExited -and [Desk]::Main($_.Id) -ne [IntPtr]::Zero
        } | Select-Object -First 1
        $null -ne $app
    } 20000)) { throw 'No window on the private desktop' }
    $null = $app.Handle # Keeps the exit code readable.
    if ($Concurrent) {
        $onlyOne = $true
        foreach ($other in $participants | Where-Object Id -ne $app.Id) {
            if (!$other.WaitForExit(12000)) { $onlyOne = $false }
        }
        Check 'concurrent relaunch leaves one application process' $onlyOne
    }
    $script:hwnd = [Desk]::Main($app.Id)
    if (!(Wait-Until { [Desk]::Child($hwnd, 'ECHO_CAPTION_TARGET') -ne [IntPtr]::Zero })) { throw 'No hit target' }
    $script:target = [Desk]::Child($hwnd, 'ECHO_CAPTION_TARGET')
    $null = Invoke-Page layout
    if (!(Wait-Until { [Desk]::Visible($target) })) { throw 'The page never placed the hit target' }
    if ($M6) { Write-Host "M6 window and layout ready after $($elapsed.ElapsedMilliseconds) ms" }
}

function Get-SubjectProcessIds {
    # A private WebView2 profile prevents unrelated applications sharing this
    # process tree. Capture descendants by PID instead of by executable name.
    $rows = @(Get-CimInstance Win32_Process -Property Name, ProcessId, ParentProcessId)
    $seen = [Collections.Generic.HashSet[int]]::new()
    $pending = [Collections.Generic.Queue[int]]::new()
    $pending.Enqueue($app.Id)
    while ($pending.Count) {
        $parent = $pending.Dequeue()
        if (!$seen.Add($parent)) { continue }
        foreach ($child in $rows) {
            if ($child.ParentProcessId -eq $parent) { $pending.Enqueue($child.ProcessId) }
        }
    }
    @($seen)
}

function Get-SubjectResources {
    $ids = @(Get-SubjectProcessIds)
    $processes = @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
    $cpu = @{}
    foreach ($process in $processes) { $cpu[$process.Id] = $process.CPU }
    $clock = [Diagnostics.Stopwatch]::StartNew()
    Start-Sleep -Milliseconds 2500
    $processes = @(Get-Process -Id $ids -ErrorAction SilentlyContinue)
    $seconds = $clock.Elapsed.TotalSeconds
    $processorSeconds = 0.0
    foreach ($process in $processes) {
        if ($cpu.ContainsKey($process.Id)) { $processorSeconds += $process.CPU - $cpu[$process.Id] }
    }
    $webviews = @($processes | Where-Object ProcessName -eq 'msedgewebview2')
    # Attribute WebView2 memory to its Chromium process role so an increase can
    # be traced to the renderer (DOM, JavaScript, decoded images), the GPU
    # process (compositor tiles and textures) or the browser process.
    $roles = [ordered]@{}
    $lines = @{}
    foreach ($row in @(Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" -Property ProcessId, CommandLine)) {
        $lines[[int]$row.ProcessId] = [string]$row.CommandLine
    }
    foreach ($process in $webviews) {
        $line = $lines[$process.Id]
        $role = if ($line -match '--type=([a-z-]+)') { $Matches[1] } else { 'browser' }
        if ($role -eq 'utility' -and $line -match '--utility-sub-type=([A-Za-z.]+)') { $role = $Matches[1].Split('.')[0] }
        $roles[$role] = [Math]::Round(([double]$roles[$role]) + $process.PrivateMemorySize64 / 1MB, 1)
    }
    @{
        webviewRolesPrivateMiB = $roles
        hostPrivateMiB = [Math]::Round(($processes | Where-Object ProcessName -ne 'msedgewebview2' | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        privateMiB = [Math]::Round(($processes | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        workingSetMiB = [Math]::Round(($processes | Measure-Object WorkingSet64 -Sum).Sum / 1MB, 1)
        webviewPrivateMiB = [Math]::Round(($webviews | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB, 1)
        cpuPercentOfOneCore = [Math]::Round(100 * $processorSeconds / $seconds, 1)
        sampleSeconds = [Math]::Round($seconds, 2)
        processCount = $processes.Count
    } | ConvertTo-Json -Compress
}

function Measure-Mode([string] $mode, [int] $cycle) {
    Start-Sleep -Seconds 2
    Write-Host "M6 profile cycle=$cycle mode=$mode $(Get-SubjectResources)"
    # Renderer-side counters from the page itself: JavaScript heap, DOM size
    # and decoded image cache. Hidden windows are measured from the outside only.
    if ($mode -ne 'hidden-local') {
        $collect = if ($RetainedSamples) { 'gc' } else { '' }
        Write-Host "M6 page cycle=$cycle mode=$mode $(Invoke-Page metrics $collect | ConvertTo-Json -Compress)"
    }
}
# The middle of the hit target, in screen coordinates.
function Get-Middle { $r = [Desk]::Window($target); @([int](($r.L + $r.R) / 2), [int](($r.T + $r.B) / 2)) }

New-Item -ItemType Directory -Path $work, "$work/native" | Out-Null
$variables = @{}
foreach ($name in 'ECHO_NATIVE_DLL_MANIFEST', 'WEBVIEW2_USER_DATA_FOLDER', 'WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS') {
    $variables[$name] = [Environment]::GetEnvironmentVariable($name)
}
Push-Location $repo
$app = $null
$started = @()
$opened = $false
try {
    $env:ECHO_NATIVE_DLL_MANIFEST = Join-Path $stage 'dlls.json'
    if (!$SkipBuild -and !$InstalledExecutable) {
        $overlay = @{ identifier = $identity; productName = 'ECHO Chrome Smoke'; mainBinaryName = 'echo-chrome-smoke'; bundle = @{ active = $false } }
        $config = Join-Path $work 'tauri-chrome.json'
        $overlay | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $config
        bun run tauri build --no-bundle --debug --features native-resources --config $config -- --locked
        if ($LASTEXITCODE -ne 0) { throw 'Chrome smoke build failed' }
    }
    if ($InstalledExecutable) {
        $binary = $InstalledExecutable
    } else {
        $binary = Join-Path $work 'echo-chrome-smoke.exe'
        Copy-Item -LiteralPath 'src-tauri/target/x86_64-pc-windows-msvc/debug/echo-chrome-smoke.exe' -Destination $binary
        foreach ($name in 'libmpv-2.dll', 'libgcc_s_seh-1.dll', 'libwinpthread-1.dll') {
            Copy-Item -LiteralPath "$stage/native/$name" -Destination "$work/native/$name"
        }
    }
    if ($Radio -and !$reuseFixture) {
        bun run fixtures:audio
        if ($LASTEXITCODE -ne 0) { throw 'Audio fixture generation failed' }
        $tone = Join-Path $work 'original tone.wav'
        Copy-Item -LiteralPath '.echo-fixtures/transport.wav' -Destination $tone
        bun scripts/seed-desktop-radio.ts $database $tone $radioMode
        if ($LASTEXITCODE -ne 0) { throw 'Could not seed the private radio profile' }
    }
    $env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $work 'webview'
    [Desk]::Open("EchoChrome$($id.Substring(0, 8))")
    $opened = $true
    Start-Subject
    [Desk]::Place($hwnd, 120, 90, 1320, 840)
    Start-Sleep -Milliseconds 900
    $dpi = [Desk]::Dpi($hwnd)
    $scale = $dpi / 96
    Write-Host "Windows $([Environment]::OSVersion.Version) dpi=$dpi"
    if ($M6) { Write-Host "M6 short idle sample: $(Get-SubjectResources)" }

    # Frame
    $style = [Desk]::Style($hwnd)
    $native = 0x40000 -bor 0x10000 -bor 0x20000 -bor 0x80000
    Check 'resize, maximize, minimize and system-menu styles survive without a caption' (($style -band $native) -eq $native) ('0x{0:x}' -f $style)
    $outer = [Desk]::Window($hwnd); $client = [Desk]::Client($hwnd)
    Check 'the page starts at the top of the frame' ($client.T - $outer.T -le 1) "$($client.T - $outer.T) px"
    Check 'invisible resize borders remain at the sides and foot' (($client.L -gt $outer.L) -and ($client.R -lt $outer.R) -and ($client.B -lt $outer.B)) "$($client.L - $outer.L), $($outer.R - $client.R), $($outer.B - $client.B)"
    $middle = [int](($outer.T + $outer.B) / 2); $centre = [int](($outer.L + $outer.R) / 2)
    Check 'the left, right and bottom borders resize' (([Desk]::Hit($hwnd, $outer.L + 2, $middle) -eq 10) -and ([Desk]::Hit($hwnd, $outer.R - 3, $middle) -eq 11) -and ([Desk]::Hit($hwnd, $centre, $outer.B - 3) -eq 15))
    Check 'the corners resize diagonally' (([Desk]::Hit($hwnd, $outer.L + 2, $outer.B - 3) -eq 16) -and ([Desk]::Hit($hwnd, $outer.R - 3, $outer.B - 3) -eq 17))
    $strip = [Desk]::Child($hwnd, 'TAURI_DRAG_RESIZE_BORDERS')
    Check 'the top edge resizes' ((([Desk]::At($centre, $client.T + 1)) -eq $strip) -and ([Desk]::Hit($strip, $centre, $client.T + 1) -eq 12))

    # Hit testing inside the band. WebView2 lays a native caption window over every
    # drag rectangle, so ask whichever window a pointer there would reach.
    $layout = Invoke-Page layout
    $hitAt = { param($spot) $x = [int]($client.L + $spot.x * $layout.scale); $y = [int]($client.T + $spot.y * $layout.scale); [Desk]::Hit([Desk]::At($x, $y), $x, $y) }
    foreach ($spot in $layout.empty) {
        $code = & $hitAt $spot
        Check "empty title bar at $([int]$spot.x),$([int]$spot.y) is a native caption" ($code -eq 2) "hit $code"
    }
    foreach ($control in $layout.controls.PSObject.Properties) {
        if (!$control.Value) { continue }
        $code = & $hitAt $control.Value
        Check "$($control.Name) is not a drag region" ($code -eq 1) "hit $code"
    }
    $at = Get-Middle
    Check 'the pointer over maximize reaches the native target' ([Desk]::At($at[0], $at[1]) -eq $target)
    Check 'it answers HTMAXBUTTON, which Snap Layouts requires' ([Desk]::Hit($target, $at[0], $at[1]) -eq 9)
    $r = [Desk]::Window($target)
    Check 'the resize strip above it still belongs to the frame' ([Desk]::Hit($target, $at[0], $r.T + 1) -eq -1)
    $drawn = $layout.maximize
    $expected = @([Math]::Round($client.L + ($drawn.x - $drawn.width / 2) * $scale), [Math]::Round($client.T + ($drawn.y - $drawn.height / 2) * $scale))
    Check 'the target covers exactly the drawn button' (([Math]::Abs($r.L - $expected[0]) -le 1) -and ([Math]::Abs($r.T - $expected[1]) -le 1) -and ([Math]::Abs($r.W - $drawn.width * $scale) -le 1) -and ([Math]::Abs($r.H - $drawn.height * $scale) -le 1)) "$($r.W)x$($r.H)"

    # Mouse messages a real pointer would produce over the empty title bar
    $spot = $layout.empty[0]
    $x = [int]($client.L + $spot.x * $layout.scale); $y = [int]($client.T + $spot.y * $layout.scale)
    $caption = [Desk]::At($x, $y)
    [Desk]::PostMouse($caption, 0xA3, 2, $x, $y)
    Check 'double-clicking the empty title bar maximizes' (Wait-Until { [Desk]::Zoomed($hwnd) })
    [Desk]::Post($hwnd, 0x112, 0xF120, 0)
    if (!(Wait-Until { ![Desk]::Zoomed($hwnd) })) { throw 'Could not restore after the double-click' }
    Start-Sleep -Milliseconds 600
    $client = [Desk]::Client($hwnd); $at = Get-Middle

    # A press reaches the page; one that leaves before release does nothing. Hover
    # needs a real pointer: without one Windows reports leaving immediately.
    $null = Invoke-Page arm
    [Desk]::Mouse($target, 0xA0, 9, $at[0], $at[1])
    Start-Sleep -Milliseconds 300
    [Desk]::Mouse($target, 0xA1, 9, $at[0], $at[1])
    Start-Sleep -Milliseconds 200
    [Desk]::Send($target, 0x2A2, 0, 0) | Out-Null
    [Desk]::Mouse($target, 0xA2, 9, $at[0], $at[1])
    Start-Sleep -Milliseconds 500
    Check 'releasing after the pointer left does not maximize' (![Desk]::Zoomed($hwnd))
    $seen = @(Invoke-Page seen)
    Check 'the press is drawn by the page and then withdrawn' ((@($seen | Where-Object { $_ -match 'pressed' }).Count -gt 0) -and ($seen[-1] -notmatch 'pressed|hovered')) ($seen -join ' | ')

    # Maximize and restore through the target
    [Desk]::Mouse($target, 0xA1, 9, $at[0], $at[1]); [Desk]::Mouse($target, 0xA2, 9, $at[0], $at[1])
    Check 'the native target maximizes' (Wait-Until { [Desk]::Zoomed($hwnd) })
    Start-Sleep -Milliseconds 700
    $area = [Desk]::Work($hwnd); $client = [Desk]::Client($hwnd)
    Check 'maximized, the page fills exactly the work area' (($client.L -eq $area.L) -and ($client.T -eq $area.T) -and ($client.R -eq $area.R) -and ($client.B -eq $area.B)) "$($client.W)x$($client.H) of $($area.W)x$($area.H)"
    Check 'the caption shows Restore' ((Invoke-Page label) -eq 'Restore')
    $at = Get-Middle
    Check 'the target follows the maximized window' (([Desk]::At($at[0], $at[1]) -eq $target) -and ([Desk]::Hit($target, $at[0], $area.T + 1) -eq 9))
    if ($Screenshots) { $null = Invoke-Page shot (Join-Path $Screenshots 'maximized.png') }
    [Desk]::Mouse($target, 0xA1, 9, $at[0], $at[1]); [Desk]::Mouse($target, 0xA2, 9, $at[0], $at[1])
    Check 'the native target restores' (Wait-Until { ![Desk]::Zoomed($hwnd) })
    Start-Sleep -Milliseconds 500
    Check 'the caption shows Maximize' ((Invoke-Page label) -eq 'Maximize')
    $now = [Desk]::Window($hwnd)
    Check 'restoring returns the previous bounds' (($now.L -eq $outer.L) -and ($now.T -eq $outer.T) -and ($now.W -eq $outer.W) -and ($now.H -eq $outer.H)) "$($now.W)x$($now.H)"
    if ($Screenshots) { $null = Invoke-Page shot (Join-Path $Screenshots 'restored.png') }

    # The same actions from the page: keyboard and assistive technology use these.
    $null = Invoke-Page press Maximize
    Check 'the drawn button maximizes' (Wait-Until { [Desk]::Zoomed($hwnd) })
    $null = Invoke-Page press Restore
    Check 'the drawn button restores' (Wait-Until { ![Desk]::Zoomed($hwnd) })
    $null = Invoke-Page press Minimize
    Check 'the drawn button minimizes' (Wait-Until { [Desk]::Iconic($hwnd) })
    [Desk]::Post($hwnd, 0x112, 0xF120, 0)
    Check 'the taskbar restore command brings it back' (Wait-Until { ![Desk]::Iconic($hwnd) })

    # System commands, as Win+arrow, the system menu and the taskbar send them
    [Desk]::Post($hwnd, 0x112, 0xF030, 0)
    Check 'the system maximize command works' (Wait-Until { [Desk]::Zoomed($hwnd) })
    [Desk]::Post($hwnd, 0x112, 0xF120, 0)
    Check 'the system restore command works' (Wait-Until { ![Desk]::Zoomed($hwnd) })

    # Sizes
    $small = [Desk]::Window($hwnd)
    $least = [Desk]::MinTrack($hwnd)
    Check 'Windows will not resize below the minimum' (($least.X -ge [int](420 * $scale)) -and ($least.Y -ge [int](540 * $scale)) -and ($least.X -le [int](460 * $scale))) "$($least.X)x$($least.Y)"
    [Desk]::Place($hwnd, 120, 90, $least.X, $least.Y); Start-Sleep -Milliseconds 700
    $client = [Desk]::Client($hwnd)
    $layout = Invoke-Page layout; $r = [Desk]::Window($target); $drawn = $layout.maximize
    Check 'the target follows the drawn button at the minimum size' (([Math]::Abs($r.L - ($client.L + ($drawn.x - $drawn.width / 2) * $scale)) -le 1) -and ([Math]::Abs($r.W - $drawn.width * $scale) -le 1)) "$($r.W)x$($r.H)"
    if ($Screenshots) { $null = Invoke-Page shot (Join-Path $Screenshots 'minimum.png') }
    foreach ($size in @(960, 1032), @(640, 1032), @(960, 516)) {
        [Desk]::Place($hwnd, 0, 0, $size[0], $size[1]); Start-Sleep -Milliseconds 500
        $layout = Invoke-Page layout; $client = [Desk]::Client($hwnd); $r = [Desk]::Window($target); $drawn = $layout.maximize
        Check "the caption stays whole in a $($size[0])x$($size[1]) snap zone" (([Math]::Abs($r.L - ($client.L + ($drawn.x - $drawn.width / 2) * $scale)) -le 1) -and ($layout.width -eq [int]($client.W / $scale)))
    }
    [Desk]::Place($hwnd, 120, 90, 1320, 840); Start-Sleep -Milliseconds 500

    # Boundary
    Check 'a modal leaves the window buttons usable' ([bool](Invoke-Page dialog))
    $refused = Invoke-Page refused
    foreach ($command in $refused.PSObject.Properties) { Check "IPC refuses $($command.Name)" ([bool]$command.Value) }
    if ($M6 -and !$ProfileWithoutMini) {
        $beforeMini = [Desk]::Window($hwnd)
        Check 'mini uses the existing WebView' ([bool](Invoke-Page mini enter))
        Check 'mini is native topmost' (Wait-Until { [Desk]::Topmost($hwnd) })
        Check 'mini ranks above a normal native peer' ([Desk]::AboveNormalPeer($hwnd))
        $mini = [Desk]::Client($hwnd)
        Check 'mini uses the compact client width' ([Math]::Abs($mini.W - 420 * $scale) -le 12) "$($mini.W)x$($mini.H)"
        Write-Host "M6 short mini sample: $(Get-SubjectResources)"
        Check 'mini remains compact after WebView reload' ([bool](Invoke-Page mini reload))
        [Desk]::Hide($hwnd)
        Check 'fixture made the original compact window hidden' (![Desk]::Visible($hwnd))
        $duplicate = Get-Process -Id ([Desk]::Start($binary, $work)) -ErrorAction SilentlyContinue
        if ($duplicate) {
            $started += $duplicate
            $exited = $duplicate.WaitForExit(12000)
        } else {
            $exited = $true
        }
        Check 'duplicate launch exits' $exited
        Check 'duplicate launch restores the original HWND' (Wait-Until { [Desk]::Visible($hwnd) })
        Check 'restored instance remains topmost' ([Desk]::Topmost($hwnd))
        Check 'return from mini uses the same WebView' ([bool](Invoke-Page mini leave))
        Check 'return from mini clears native topmost' (Wait-Until { ![Desk]::Topmost($hwnd) })
        $restoredMini = [Desk]::Window($hwnd)
        Check 'return from mini restores the original rectangle' (($restoredMini.L -eq $beforeMini.L) -and ($restoredMini.T -eq $beforeMini.T) -and ($restoredMini.W -eq $beforeMini.W) -and ($restoredMini.H -eq $beforeMini.H))
    }
    if ($Radio) {
        bun scripts/test-desktop-radio.ts $port $radioMode
        Check 'private WebView2 radio journey' ($LASTEXITCODE -eq 0)
    }

    if ($ProfileCycles) {
        if ($ProfileWithoutMini) { Write-Host 'Profiling without mini mode. Native mini acceptance is outside this run.' }
        $info = [Diagnostics.ProcessStartInfo]::new((Get-Command bun).Source)
        $info.CreateNoWindow = $true
        $info.UseShellExecute = $false
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        foreach ($argument in 'scripts/test-window-chrome.ts', "$port", 'session') {
            $info.ArgumentList.Add($argument)
        }
        $script:profileDriver = [Diagnostics.Process]::Start($info)
        $soak = [Diagnostics.Stopwatch]::StartNew()
        for ($cycle = 1; $cycle -le $ProfileCycles; $cycle++) {
            $null = Invoke-Page profile idle
            Measure-Mode home-idle $cycle
            $null = Invoke-Page profile local
            Measure-Mode home-local $cycle
            if (!$ProfileWithoutMini) {
                $null = Invoke-Page mini enter
                Measure-Mode mini-local $cycle
                $null = Invoke-Page mini leave
            }
            $null = Invoke-Page profile radio
            Measure-Mode radio $cycle
            $null = Invoke-Page profile local
            [Desk]::HideToTray($hwnd)
            Measure-Mode hidden-local $cycle
            [Desk]::ShowFromTray($hwnd)
            $null = Invoke-Page profile library
            Measure-Mode library $cycle
            Write-Host "M6 retained cycle=$cycle elapsed=$([Math]::Round($soak.Elapsed.TotalSeconds)) $(Invoke-Page metrics gc | ConvertTo-Json -Compress)"
        }
        $null = Invoke-Page profile idle
        Measure-Mode home-final $ProfileCycles
        $null = Invoke-Page pressure
        Measure-Mode home-after-pressure $ProfileCycles
        # Repeated idle samples check stability within the same instrumentation session.
        foreach ($repeat in 1..3) {
            Write-Host "M6 page repeat=$repeat $(Invoke-Page metrics | ConvertTo-Json -Compress)"
        }
        Stop-ProfileDriver
        Write-Host "M6 profile elapsed: $([Math]::Round($soak.Elapsed.TotalSeconds, 1)) seconds"
        $profileProcessIds = @(Get-SubjectProcessIds)
    }

    # Closing runs the existing bounded shutdown
    if ($M6) {
        $area = [Desk]::Work($hwnd)
        [Desk]::Place($hwnd, $area.L + 24, $area.T + 24, [int](800 * $scale), [int](620 * $scale))
        $placement = [Desk]::Window($hwnd)
    }
    [Desk]::Post($hwnd, 0x112, 0xF060, 0)
    Check 'the system close command (Alt+F4) ends the process cleanly' ($app.WaitForExit(12000) -and $app.ExitCode -eq 0) "exit $($app.ExitCode)"
    if ($ProfileCycles) {
        Check 'sampled process tree exits after profiling' (Wait-Until {
            @(Get-Process -Id $profileProcessIds -ErrorAction SilentlyContinue).Count -eq 0
        } 12000)
    }
    Start-Subject -Concurrent:$M6
    if ($M6) {
        $restored = [Desk]::Window($hwnd)
        $fits = $area.W -ge 848 * $scale -and $area.H -ge 668 * $scale
        $withinArea = ($restored.L -ge $area.L) -and ($restored.T -ge $area.T) -and ($restored.R -le $area.R) -and ($restored.B -le $area.B)
        Check 'normal placement stays on the original display after relaunch' $withinArea
        if ($fits) {
            $same = ($restored.L -eq $placement.L) -and ($restored.T -eq $placement.T) -and ([Math]::Abs($restored.W - $placement.W) -le 2) -and ([Math]::Abs($restored.H - $placement.H) -le 2)
            Check 'clean relaunch restores saved normal placement' $same
        }
    }
    Invoke-Page press Close | Out-Null
    Check 'the drawn close button ends the process cleanly' ($app.WaitForExit(12000) -and $app.ExitCode -eq 0) "exit $($app.ExitCode)"
} finally {
    Stop-ProfileDriver
    foreach ($process in $started) { if (!$process.HasExited) { $process.Kill($true); $process.WaitForExit(5000) | Out-Null } }
    if ($opened) { [Desk]::Close() }
    Pop-Location
    foreach ($name in $variables.Keys) { [Environment]::SetEnvironmentVariable($name, $variables[$name], 'Process') }
    Start-Sleep -Milliseconds 1200
    $cleanupPaths = if ($reuseFixture) { @($work) } else { @($work, $database) }
    foreach ($path in $cleanupPaths) {
        if (Test-Path -LiteralPath $path) {
            $resolved = (Resolve-Path -LiteralPath $path).ProviderPath
            if ($resolved -ne [IO.Path]::GetFullPath($path) -or
                (Get-Item -LiteralPath $resolved).Attributes.HasFlag([IO.FileAttributes]::ReparsePoint)) {
                throw 'Unexpected private desktop profile path'
            }
            Remove-Item -LiteralPath $resolved -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
$failed = @($results.Keys | Where-Object { !$results[$_] })
Write-Host "$($results.Count - $failed.Count) of $($results.Count) title bar checks passed"
if ($failed.Count) { throw "Failed: $($failed -join '; ')" }
