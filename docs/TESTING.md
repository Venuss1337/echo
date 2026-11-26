# Testing

ECHO is tested from the command line. Almost everything runs without a
visible window, a mouse, an audio device, public radio stations or personal
music. What genuinely needs a person or hardware is listed at the end as a
manual checklist.

## Everyday commands

Run from the repository root in PowerShell 7 after
[Development](DEVELOPMENT.md) setup.

| Command | What it runs |
| --- | --- |
| `bun run verify` | Production build of the renderer; ESLint, Prettier, TypeScript, contract drift; `cargo fmt`, Clippy with warnings denied, all Rust tests; Vitest; Playwright. This is the gate for every pull request. |
| `bun run check` | Lint, format, types and contract drift only. |
| `bun run test:unit` | Vitest and Testing Library component tests. |
| `bun run test:e2e` | Build and Playwright in headless Chromium against a contract-accurate fake backend (`tests/browser/support/library-fixture.ts`), including visual baselines. |
| `bun run test:contracts` | Fails if `src/generated/contracts.ts` differs from the Rust types. |
| `bun run test:db` | Every `library::` Rust test on real temporary databases and directories. |
| `bun run test:library-scale` | Timed 10,000- and 20,000-track benchmarks (`ECHO_BENCH_OUT=<file>` appends JSON lines). |
| `bun run test:native` | The libmpv FFI against the official mpv test DLL on null output. Needs `$env:ECHO_VERIFIED_LIBMPV = & ./scripts/prepare-native-probe.ps1` first. |
| `bun run test:security` | `bun audit`, `cargo audit`, and the license and source policy in `deny.toml`. Run it from PowerShell. |

`cargo test --locked --manifest-path src-tauri/Cargo.toml <filter>` runs a
subset of Rust tests, for example `radio::` or `playback::history`.

## Layers

| Layer | Where | Proves |
| --- | --- | --- |
| Rust unit and property tests | beside each module | State machine and queue invariants, generation races, URL and address policy, migrations, scanning, user data rules |
| Native decoder | `bun run test:native`, `native/windows-x64/test-local.ps1`, `test-live.ps1` | Real libmpv events, seeking, errors and shutdown on null output; radio transport through the real decoder |
| Browser | `tests/browser/*.spec.ts` | Every view, keyboard and focus behaviour, menus and dialogs, contrast under extreme covers, idle behaviour, view retention, visual baselines |
| Real window | `native/windows-x64/test-window-chrome.ps1` | The production window in real WebView2 on a private desktop |
| Installer | `native/windows-x64/smoke-installer.ps1` | Install, verify the decoder and notices from the installed path, run the native suite there, uninstall |
| Manual | [below](#manual-acceptance) | Audible output, devices, shell integration, assistive technology |

Headless results never prove audible sound, a physical device change or
what a screen reader announces.

## Native suites

With a staged decoder ([Native decoder](NATIVE.md)):

- `test-local.ps1 -ResourceRoot $stage` plays generated WAV, MP3, FLAC, Ogg
  Vorbis, M4A AAC and ADTS AAC on null output: pause and resume, seek within
  ±250 ms, mixer, stop, end of file, queue advance, stale-seek rejection,
  replacement, session restart, corrupt and unsupported files, and rejected
  playlist, concat and external references with a loopback listener that
  must see no connection. `-Measure` adds an idle and null-output resource
  sample; `-VerifyDefaultOutput` opens the real default device, muted.
- `test-live.ps1 -ResourceRoot $stage` streams controlled MP3 and AAC through
  the Rust connector into the decoder: restart, source switches, pause and
  resume, reconnect and exhaustion, misleading MIME types and stalled
  shutdown. `-Measure -DurationSeconds 600` runs a paced soak.
- `test-library-playback.ps1` plays a library track during a rescan and
  after its file is removed.
- `test-no-output.ps1` runs where Windows reports no active render endpoint
  (the GitHub-hosted runner, via `native-candidate.yml` with
  `verify_no_output`) and expects a typed output failure and clean shutdown.

## Real window on a private desktop

`test-window-chrome.ps1 -ResourceRoot $stage` builds the app under a
disposable identity and runs it on a separate Windows desktop: nothing
appears on screen, no pointer moves and no focus changes, and it works while
the session is locked. It checks frame styles, resize and caption hit tests,
`HTMAXBUTTON`, maximize, restore, minimize, minimum size, snap-zone sizes,
modals and refused IPC.

- `-M6` adds the mini player (same WebView, topmost, restored rectangle),
  duplicate launch, placement restore and both ways of closing.
- `-Radio -FixtureRadio` adds the real radio journey against this
  repository's fixture files over public HTTPS. The files are compared byte
  for byte with the checkout first; set `ECHO_RADIO_FIXTURE_ORIGIN` to test
  from a fork.
- `-ProfileCycles <n>` adds repeated resource sampling; see
  [Performance](PERFORMANCE.md).

`test-installed-radio.ps1` runs the same against a release build installed
under a disposable identity. `test-desktop-playback.ps1 -Journey` drives the
complete journey (folder picker, scan, browse, search, playback, playlists,
favorites, compact, restart, root removal) and checks every music file is
byte-identical afterwards; it needs an interactive desktop with an audio
endpoint. `test-window-input.ps1` sends real input and refuses to run on a
desktop that is in use.

## Visual baselines

Playwright baselines render the bundled fonts at fixed sizes with reduced
motion and the field's grain hidden. They change only for an intentional,
reviewed change: `bunx playwright test --update-snapshots`, then inspect
every PNG diff before committing. `bun scripts/capture-visuals.ts dist <out>`
renders review captures of every view.

## Rules

- Tests are deterministic: generated fixtures, fixed seeds, no wall-clock
  sleeps as synchronisation, no public stations, no user data.
- Test audio is synthesized ([fixtures](../tests/fixtures/audio/README.md)).
- A test that cannot run is reported as not run, with the reason. It is never
  skipped, loosened or deleted to make a build green.

## Manual acceptance

These checks need hardware or a person. Use a dedicated, non-elevated
Windows account or a virtual machine, the release installer, a local file
you own and a station you are allowed to play. Record the Windows and
WebView2 versions, the installer checksum, audio endpoints, displays and
scaling. Attach the tray's **Export diagnostics** snapshot to any failure.

| Area | Steps | Expected |
| --- | --- | --- |
| Install and upgrade | Install the previous release, add a folder, make a playlist and favorites, play something. Install the new release over it. Uninstall, keeping data. Reinstall. | Everything survives the upgrade and the reinstall; music files are untouched; no administrator prompt. |
| Audible playback | Play local MP3, FLAC and AAC files and a station at a comfortable volume. Seek, pause, skip. | Sound matches the interface; position and state agree. |
| Output devices | During local playback change the default device in Windows settings; unplug the active device; reconnect it; press Play. Repeat with a USB or Bluetooth device and with a station. | The interface shows an output failure rather than playing silently; Play resumes local audio at the last position and radio at the live edge. |
| Tray and mini player | Hide to tray, show, open the mini player over another window, minimize and restore it, launch ECHO again while hidden, exit from the tray. | Playback continues while hidden; the mini player stays on top; a second launch shows the running window in its mode; exit removes the tray icon and media session. |
| Media controls | Use the Windows media flyout and physical media keys for play, pause, next, previous and seek; switch between local and radio quickly. | Title, artist, album and art follow the current source; radio shows no timeline; stale artwork or seeks never apply to the new source. |
| Window | Drag, resize edges and corners, maximize, Snap Layouts, `Win+Z`, `Alt+Space`, `Alt+F4`, at 100 %, 125 %, 150 % and 200 %, and across monitors with different scales. Disconnect the monitor ECHO was on and relaunch. | Hit areas match the drawn controls; the window always reopens on an available display. |
| Accessibility | Keyboard only through every view, the player, dialogs and the mini player; Narrator or NVDA; a Windows contrast theme; reduced motion. | Visible focus, logical order, focus restored after dialogs, clear names and states, no motion with reduced motion. |
| Clean exit | Exit during a scan, during radio connection and while minimized. | The process ends within ten seconds and the next start is clean. |
