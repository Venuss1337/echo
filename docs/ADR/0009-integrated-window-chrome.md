# ADR 0009 — Integrated window chrome

**Status:** accepted for the Atmosphere 2.0 task, 2026-11-24. Windows only.

## Context

ECHO stacked a 63 px breadcrumb header under the native Windows caption. The
redesign needs one title bar that belongs to the application, without giving up
native dragging, resizing, snapping, the system menu or Snap Layouts, and without
handing the page general power over its window.

## Decision

1. **Frame.** `decorations: false, shadow: true`. tao keeps `WS_THICKFRAME`,
   `WS_MAXIMIZEBOX`, `WS_MINIMIZEBOX` and `WS_SYSMENU` and answers
   `WM_NCCALCSIZE`, so resizing, Aero Snap, Win+arrow, the shadow and the
   work-area maximize (including auto-hide taskbars) stay native.
2. **Dragging.** CSS `app-region: drag` and nothing else. wry enables WebView2's
   non-client region support, and WebView2 answers by creating one native
   `HTCAPTION` window per drag rectangle, tiled around `no-drag` controls. That
   brings dragging, double-click maximize and the system menu with **no IPC
   permission**. `core:window:allow-start-dragging` is deliberately not granted.
3. **Snap Layouts.** Windows 11 offers them only where a hit test answers
   `HTMAXBUTTON`, and the page covers the whole client area. `window.rs` places a
   small child window over the drawn maximize button that answers exactly that,
   paints nothing, and reports hover and press to the page. Windows Terminal uses
   the same arrangement. The strip above the button still returns
   `HTTRANSPARENT`, so the top edge keeps resizing.
4. **Actions.** Three commands, main window only: `window_subscribe` (state
   channel), `window_act` (a bare string: `minimize`, `toggleMaximize`, `close`)
   and `window_caption` (where the maximize button is drawn). No `core:window`
   permission is granted.
5. **Closing.** The close button calls `window.close()`, so the existing
   checkpoint, decoder teardown and ten-second bound run unchanged.
6. **Failure.** The caption is rendered outside the React error boundary, so a
   shell that fails to render still has its window buttons. Modals are no longer
   native `<dialog>` elements, because `showModal()` would make the caption inert.

## Invariants

- `window_caption` accepts only a rectangle that could be a caption button near
  the top-right corner (16–96 by 16–64 px, at most 240 px from the right and
  32 px from the top, finite). The page cannot use it to cover other parts.
- Bounds are CSS pixels from the right edge. Edges, not sizes, are scaled by
  `GetDpiForWindow`, so neighbouring buttons never overlap at fractional scales.
- State is sent to the page under the lock and as it is at that moment. Tasks may
  run in any order; the last message delivered is never older than the truth.
- The window procedure never re-enters the runtime from inside a message.

## Alternatives

- **`tauri-plugin-frame`, `tauri-plugin-snap-layout`:** the same mechanism, but an
  extra dependency that injects scripts, emits global events and positions by
  arithmetic that must match CSS.
- **Simulated Win+Z on hover:** fragile and focus-dependent.
- **Subclassing Chromium's windows:** another process owns them.
- **`data-tauri-drag-region`:** needs IPC permission and does not serve touch.

## Verification

`native/windows-x64/test-window-chrome.ps1` runs the production window in real
WebView2 on a private Windows desktop (53 checks, no pointer, no focus, nothing
visible). `test-window-input.ps1` covers what only real input can show; it refuses
to run on a desktop that is in use or locked, and refuses any click or key that
would reach another window.

**Not run:** physical dragging and edge resizing, the visible Snap Layouts
flyout, the two system menus, drawn hover, mixed-DPI monitors. See
[Testing](../TESTING.md#atmosphere-20-tests).

## Reversal triggers

WebView2 documenting caption-button regions for windowed hosting would remove the
need for the child window. A Tauri regression in undecorated frames would justify
returning to native decorations; `TitleBar.tsx` and `window.rs` are the only
parts that would change.
