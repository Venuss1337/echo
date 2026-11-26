# Profiling ECHO

Measure the installed app as a process tree. ECHO owns audio and storage,
while WebView2 uses separate browser, renderer, GPU, network, storage and
crash reporting processes. Record the Windows and WebView2 versions, DPI,
display refresh rate, power mode, build, fixture library and other load.

## Repeatable runs

- Run `bun run test:library-scale` for SQLite query and index measurements,
  and `bunx playwright test tests/browser/library.spec.ts` for large-library
  renderer navigation and virtualization.
- Run `native/windows-x64/test-installed-radio.ps1 -ResourceRoot <stage>
  -M6 -FixtureRadio -ProfileCycles 45` for an installed playback and
  navigation soak on a private desktop. The fixture covers Home, local audio,
  radio, mini-player, library and a minimize-then-hide phase. It exercises no
  Explorer tray interaction. See [Testing](TESTING.md) for the reviewed
  decoder stage and fixture origin.
- Use `test-window-chrome.ps1 -M6 -Radio -FixtureRadio -ProfileCycles 6`
  for a smaller reproduction on a debug build. Supply the same stage.
- Capture startup separately as time to a ready title bar. Measure fresh
  app profiles and warmed relaunches separately, and keep fixture and build
  settings fixed when comparing versions.

Private bytes measure committed memory particular to each process. Summed
working sets can count shared pages more than once. CPU is reported as a
percentage of one logical core over about 2.5 seconds. GPU process private
memory is separate from hardware GPU utilization and dedicated video memory.

## Interpreting renderer samples

The profiler separately reports JavaScript heap size, listener and node
counts, composited layer area and loaded image dimensions. Process
measurements are taken before the page probes. The soak reuses one
Playwright connection to limit observation overhead, and measures retained
objects after GC once per cycle. Forced GC and memory pressure run only in
the private test harness.

For a suspected leak, collect samples after warm-up at the same point in
each cycle. Compare the retained JavaScript counters with native and GPU
process private bytes. A transient peak or a cache that responds to simulated
pressure does not establish unbounded growth. Use a sustained run, an idle
control, and teardown checks before attributing growth to a feature.

The minimized window tells WebView2 to stop rendering and asks supported
runtimes to trim caches. The page remains connected to the Rust playback
coordinator. See [Windows integration](WINDOWS.md#tray).
