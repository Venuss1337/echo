# Playback and the native decoder

Read `docs/PLAYBACK.md` and `docs/CONTRACTS.md` first.

- `service.rs` is the single owner of the decoder, the queue and the session
  checkpoint. Every state change goes through its bounded mailbox, including
  the tray, system media controls and the renderer.
- A command returning `Ok` means accepted, not audible. Only a native event
  for the current source generation may move the state to Playing; events
  from an older generation are discarded.
- `decoder.rs` is the only FFI to libmpv. Every `unsafe` block carries a
  `// SAFETY:` comment covering ownership, lifetime, threading and teardown.
  Never pass a path, URL or option string from IPC to the decoder.
- `source.rs` reopens and re-authorizes local files on every load. Do not
  cache a path decision across loads or follow reparse points.
- The decoder is the reviewed callback-only build. Enabling a demuxer,
  protocol or option changes its security boundary: that needs a new
  decoder review (`docs/NATIVE.md`), not a code change alone.
- Tests: pure state and queue tests run anywhere; native behaviour is proven
  with `bun run test:native` and `native/windows-x64/test-local.ps1` on null
  output. Null output never proves audible playback.
