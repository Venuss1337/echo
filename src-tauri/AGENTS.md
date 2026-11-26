# Rust host and Windows integration

Read `docs/ARCHITECTURE.md`, `docs/WINDOWS.md` and `docs/CONTRACTS.md` first.

- `lib.rs` composes services; each domain owns its thread or worker and its
  shutdown. Exit stops playback first, checkpoints, then tears down within
  the bounded shutdown.
- Window, tray and system media controls are projections of confirmed state
  and send commands to the same playback mailbox as the page. They never own
  a decoder or a second copy of the queue.
- Window commands accept only the fixed `WindowIntent` values. No IPC may set
  sizes, positions, z-order or HWNDs; no `core:window` permission is granted.
- Native Windows calls run on the thread that owns the window. Explain
  ownership and lifetime for every `unsafe` block.
- Real-window behaviour is verified with
  `native/windows-x64/test-window-chrome.ps1` on a private desktop.
