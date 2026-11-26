# Windows integration and lifecycle

ECHO is a Windows x64 application. This document covers what it does with
the operating system around the page: its window and title bar, the mini
player, the tray, system media controls, a second launch, diagnostics and
startup and shutdown. The decisions are recorded in
[ADR 0009](ADR/0009-integrated-window-chrome.md) and
[ADR 0011](ADR/0011-windows-integration.md).

## The window and title bar

Tauri creates the window with `decorations: false` and `shadow: true`. The
native frame stays: resizing, Aero Snap, Win+arrow, the shadow and a
work-area maximize that respects auto-hide taskbars all behave as in any
Windows application.

- **Dragging** comes from CSS `app-region: drag`. WebView2 turns each drag
  rectangle into a native caption region, so dragging, double-click maximize
  and the system menu work with no IPC permission.
- **Snap Layouts** appear only where a hit test answers `HTMAXBUTTON`, and the
  page covers the client area. `src-tauri/src/window.rs` places a small,
  invisible child window over the drawn maximize button that answers exactly
  that and reports hover and press back to the page, the arrangement Windows
  Terminal uses. The strip above it still belongs to the frame, so the top
  edge keeps resizing.
- **Commands.** The page has three: `window_subscribe` (a state channel),
  `window_act` (one of `minimize`, `toggleMaximize`, `close`, `enterMini`,
  `returnFromMini`) and `window_caption` (where the maximize button is drawn,
  validated to be a plausible caption button near the top-right corner). No
  `core:window` permission is granted; the page cannot size, move, re-order
  or create windows.
- The caption buttons render outside the React error boundary, so a failed
  shell still has working window buttons.

## Mini player

The mini player is the same window, WebView and subscriptions presented at
420 × up to 640 logical pixels, always on top, in the corner of the current
work area. Rust computes the bounds; the page only asks to enter or leave.
Entering records the exact Windows placement, including restore bounds of a
maximized window, and leaving restores it. It refuses a work area too small
for the compact layout. ECHO always starts in normal mode and remembers
normal-window geometry between runs, falling back to the default placement
when the saved display is gone.

## Tray

The tray icon offers playback controls, **Show ECHO**, the mini player,
**Hide to tray**, **Export diagnostics** and **Exit ECHO**. Closing the window
still exits; hiding to the tray is an explicit choice. Tray actions enter the
same bounded playback mailbox as the page.

While the window is minimized or hidden in the tray, playback, the tray and
the system media controls continue in Rust. ECHO tells WebView2 the page is
not visible (`ICoreWebView2Controller::IsVisible`) and sets its memory target
to low (`ICoreWebView2_19::MemoryUsageTargetLevel`), so the renderer stops
drawing and WebView2 can release graphics memory and caches. Restoring the
window reverses both. The page keeps running throughout, so no state is lost.

## System media controls

`src-tauri/src/media_session.rs` owns a WinRT System Media Transport Controls
session on a dedicated thread. It is a projection of confirmed playback:

- title, artist, album and a bounded, cached thumbnail for local tracks;
- station name and cached station artwork for radio, with no timeline;
- a seekable timeline only for seekable local playback.

Play, pause, next, previous, stop and seek from the media flyout or physical
media keys become commands on the playback mailbox; seeks carry the source
generation, so a seek aimed at a replaced track is ignored. Metadata loaded
for an old source is discarded, and the session is cleared at exit.

## A second launch

The pinned official single-instance plugin runs before setup with the
application identifier as its namespace. A second launch exits and the
running instance restores its window in its current mode. The forwarded
command line and working directory are discarded; nothing is opened or
fetched on their behalf.

## Diagnostics

**Export diagnostics** in the tray writes a small JSON snapshot to ECHO's
local application data: version, platform, schema version and readiness
flags. It contains no paths, station URLs, track names or free-form logs,
and it is never sent anywhere.

## Startup and shutdown

1. The SQLite bootstrap and decoder verification run on an owned worker
   while the window appears; the page shows the library as opening.
2. Packaged builds verify every decoder DLL against the manifest compiled
   into the executable before playback is offered.
3. Startup restores the library, the queue and mixer settings in an inert
   state. Nothing plays and no station connects until the user asks.
4. On exit, playback stops first so its last listening event is recorded,
   the session is checkpointed, then the decoder is torn down. The process
   ends after a ten-second bound even if a native call is stuck.

## Data locations

ECHO keeps its database, artwork cache, settings and diagnostics in the
per-user application data directory for its identifier
(`%LOCALAPPDATA%\org.echo.player`) and WebView2 data beside it. Music
folders are only read. Uninstalling leaves user data in place unless the
user chooses to delete it in the uninstaller.

## Verification

- `native/windows-x64/test-window-chrome.ps1` runs the real window in real
  WebView2 on a private Windows desktop: frame styles, hit tests, maximize,
  restore, minimize, minimum size, snap-zone sizes, modals, refused IPC,
  the mini player, a concurrent second launch, placement restore and both
  ways of closing. It moves no pointer, takes no focus and works while the
  session is locked.
- `native/windows-x64/test-window-input.ps1` covers what needs real input
  (dragging, edge resizing, the Snap Layouts flyout, system menus). It
  refuses to run on a desktop that is in use.
- Physical media keys, the visible media flyout, the Explorer tray menu,
  mixed-DPI monitors and assistive technology are checked by a person; see
  the [manual acceptance checklist](TESTING.md#manual-acceptance).
