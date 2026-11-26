# Playback

ECHO plays local files and internet radio through one native engine, libmpv,
driven by one Rust owner. The renderer never plays audio and never sees a
path or URL it could hand to the decoder.

## Ownership

| Module (`src-tauri/src/`) | Responsibility |
| --- | --- |
| `playback.rs` | The state machine: states, transitions and the effects they request. Pure and testable without a device. |
| `playback/queue.rs` | Stable entry IDs, ordering, repeat and shuffle traversal. |
| `playback/service.rs` | The Windows worker: a bounded mailbox, the current native core, checkpoints and history hooks. The only place playback state changes. |
| `playback/decoder.rs` | The minimal FFI to libmpv. Only calls ECHO uses; every `unsafe` block documents its invariants. |
| `playback/source.rs` | Opens approved local files and serves their bytes to the decoder through a one-use callback. |
| `playback/persistence.rs` | Queue and mixer checkpoints in SQLite. |
| `playback/history.rs` | Turns confirmed playback into listening events. |
| `playback/desktop.rs` | The main-window commands and the native audio file picker. |

Radio sources come from `radio/` and enter the same worker; see
[Radio](RADIO.md). The tray and Windows media controls send commands to the
same mailbox; see [Windows integration](WINDOWS.md).

## State and generations

States are `idle`, `loading`, `playing`, `paused`, `buffering`, `stopping`,
`ended` and `error`. Every load starts a new **source generation** and a new
native core. Events carry the generation they belong to, and events from an
older generation are discarded, so a late end-of-file or error from a
replaced track can never advance or break the current one.

- A command returning success means the worker accepted it. Only a native
  playback-restart event for the current generation makes the state
  `playing`; the page never shows playing on its own.
- Stop waits for the matching terminal event. Natural end-of-file advances
  exactly once and applies repeat.
- A decode or output failure stops on the affected entry with a typed error
  and waits for the user. There is no automatic retry loop for local files.
- Seek is accepted only for seekable sources and carries the generation it
  was aimed at.
- Volume and mute are shared by local and radio playback and persisted.

The worker waits on libmpv wakeups instead of polling and samples the
authoritative position at most twice a second. The renderer interpolates the
needle locally only while confirmed playing and visible.

## Queue

Entries have stable identities, including duplicates of the same track.
Repeat is off, one or all. Shuffle keeps the visited prefix and a stable
order until the user changes it; turning it off restores the original order.
Library actions (play, play next, add to queue) hand resolved items to the
worker and queue at most 1,024 entries, windowed around the chosen row.
Checkpoints are written at most every ten seconds during playback and on an
orderly exit. On the next start the queue and mixer are restored in an inert
state; Play resumes the restored entry at its checkpoint when available.

## The decoder and its boundary

ECHO ships one libmpv build, the reviewed `local` profile of mpv 0.42.0 and
FFmpeg 9.1 compiled from pinned sources ([Native decoder](NATIVE.md)). It is
deliberately small:

- FFmpeg networking and every URL protocol are compiled out. Demuxers are
  limited to AAC, FLAC, MOV/MP4, MP3, Ogg and WAV.
- mpv's stream registry is patched to the read-only callback handler only,
  and its demuxer registry to lavf. Configuration files, external
  references, auxiliary media, video output and scripting are disabled.
- The decoder never opens a file or URL itself. For local playback,
  `source.rs` opens the file and serves bytes through a fixed opaque
  callback URI registered for one generation. For radio, the Rust connector
  does the same with network bytes.
- At load the adapter checks that the active core advertises no built-in
  protocols, so a different DLL cannot silently restore them.

This removes the decoder's own network and secondary-file access. It does
not sandbox native code: a memory-safety bug in a parser would still run in
ECHO's process. Keeping the decoder current and its inputs bounded is the
mitigation. See [ADR 0006](ADR/0006-local-decoder-boundary.md).

### Local file access

Files come only from the native file picker or an indexed library folder.
On every load, the host re-authorizes the path: it must be on a local drive,
resolve through the volume GUID, and contain no UNC, device or alternate
stream syntax. Each component is opened without following reparse points,
and the checked ancestors stay open, denying rename and delete, until the
file itself is held read-only. A file that moved or became a junction since
it was indexed is refused.

### Loading the DLLs

Packaged builds embed the SHA-256 manifest of the three decoder DLLs. Before
playback is offered, the host checks the exact file set, digests, client
ABI and required exports, loads them from the installation directory with
dependency search limited to that directory and System32, and keeps the
files open to deny replacement while mapped. Development builds without a
staged decoder run with audio disabled.

## Output

Playback uses the default WASAPI endpoint with null fallback disabled. If the
output cannot be opened, or disappears, playback stops with a typed
`outputUnavailable` error rather than pretending to play. Play retries with a
fresh core against the current default device and resumes at the last
confirmed position. Default-device changes during playback are handled inside
mpv's WASAPI backend.

## Not supported

Gapless transitions between tracks, crossfade, ReplayGain, equalizer and
visualizations are not implemented. Formats outside the six demuxers above
are rejected with an unsupported-media error.

## Tests

- `cargo test --locked --manifest-path src-tauri/Cargo.toml` covers the state
  machine, queue invariants and races without a device.
- `bun run test:native` and `native/windows-x64/test-local.ps1` play
  generated WAV, MP3, FLAC, Ogg Vorbis, M4A and ADTS AAC through the real
  decoder on null output: pause, resume, seek within ±250 ms, stop, end of
  file, queue advance, stale-seek rejection, replacement, corrupt and
  unsupported files, playlist and concat indirection, and shutdown.
- `test-local.ps1 -VerifyDefaultOutput` opens the real default device muted.
  No automated test proves audible sound; that is a
  [manual check](TESTING.md#manual-acceptance).
