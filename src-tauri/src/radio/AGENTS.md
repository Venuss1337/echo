# Internet radio transport

Read `docs/RADIO.md`, `docs/SECURITY.md` and `docs/ADR/0010-radio-connector.md` first.

- Rust fetches every station and every artwork byte. The decoder only reads
  an opaque one-use `echo-radio://` callback stream; it never sees a URL.
- Each hop is parsed, resolved, and pinned to an address that passed the
  public-unicast policy in `net.rs`. A redirect, retry or new DNS answer
  starts validation again. Do not add a second resolution, a proxy, or a
  code path that connects without the pinned address.
- Timeouts, byte limits and queue sizes are part of the security boundary.
  Changing one needs a test that shows the new bound.
- Loopback and private-address permissions exist only in the unit-test
  build (`fixtures.rs`). Never relax production policy to make a test or a
  station work.
- Directory data, ICY metadata and artwork are hostile input: bound,
  sanitize and re-encode them before they reach SQLite or the renderer.
- Tests: `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib radio::`
  and `native/windows-x64/test-live.ps1`. Tests never depend on public
  stations.
