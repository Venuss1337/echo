# Security model

This document describes what ECHO protects, where its trust boundaries are
and how each is enforced. To report a vulnerability, follow
[SECURITY.md](../SECURITY.md) at the repository root instead.

## Assets and assumptions

ECHO protects the user's music files (never modified or deleted), their
local data (library, playlists, favorites, history), and the user's machine
and network from content it plays: files, tags, artwork, directory data and
radio streams are all treated as hostile. The installation directory and
the user's own account are trusted; ECHO does not defend against an attacker
who already controls them.

## Boundaries

| Boundary | Threat | Control |
| --- | --- | --- |
| Renderer → host IPC | A compromised or buggy page using native power | One capability with an explicit command list; no plugin, file system, shell, dialog, HTTP or `core:window` permissions; IDs and bounded values only; Rust validates every argument. |
| Renderer content | Script injection | CSP `default-src 'self'`; scripts and styles from the bundle only; images from the bundle or `blob:`; no remote pages, no `eval`, no inline script in production. |
| Local files | Path traversal, junction and symlink escapes, swapped files | Files come only from native pickers or indexed roots. Every open re-authorizes a local-drive path, walks components without following reparse points and holds ancestors open against rename. UNC, device and alternate-stream paths are refused. Music files are opened read-only. |
| Tags and artwork | Parser bugs, memory exhaustion | Length caps and control-character removal for text; header-checked dimensions, pixel and allocation limits and one decode at a time for images; JPEG and PNG only. |
| Decoder | Network or secondary file access, DLL substitution | A callback-only libmpv build with FFmpeg networking and URL protocols compiled out; bytes are served by Rust. DLLs are verified by digest, ABI and exports from the installation directory before use, with dependency search limited to that directory and System32. |
| Radio | SSRF to local services, DNS rebinding, TLS downgrade | The Rust connector resolves and pins public unicast addresses per hop, refuses local interfaces, disables proxies, redirects and retries in the client, verifies TLS offline and never downgrades. See [Radio](RADIO.md). |
| Directory and ICY data | Malicious metadata | Size-limited, validated row by row, sanitized before storage and display; URLs never reach the page. |
| SQLite | Injection, data loss | Bound parameters only; atomic forward-only migrations; corrupt or newer databases are reported, never reset; scans flag, never delete. |
| Window | Spoofing or hiding other windows | Five fixed window intents; Rust computes every size and position; the caption rectangle is validated to a plausible button. |
| Second launch | Argument injection | The forwarded command line and working directory are discarded. |
| Diagnostics | Leaking personal data | An explicit native action writes an allowlisted local snapshot without paths, URLs, titles or logs. Nothing is sent anywhere. |
| Supply chain | Compromised dependency or build | Lockfiles; registry and license policy (`deny.toml`); `bun audit` and `cargo audit`; GitHub Actions pinned to commit SHAs with read-only default permissions; the decoder rebuilt from hash-checked sources and compared with committed inventories. |

## What is not protected

- **Native code safety.** A memory-safety bug in libmpv, FFmpeg, Lofty or an
  image decoder runs inside ECHO's process. The decoder's inputs are
  narrowed and bounded, but it is not sandboxed. Keeping these components
  current is the mitigation.
- **A stalled disk.** A decoder blocked in a synchronous read on a hung
  driver cannot always be cancelled; exit is bounded at ten seconds by
  ending the process.
- **Broadcaster privacy.** Listening to a station necessarily reveals the
  user's IP address to that broadcaster and, when searching, to Radio
  Browser. See [PRIVACY.md](../PRIVACY.md).
- **Signed binaries.** Installers are not code-signed today; integrity rests
  on GitHub Releases, published SHA-256 checksums and build attestations.
- **Online revocation.** TLS for radio is verified offline against bundled
  roots, without revocation checks.

## Changes that need a security review

Anything that adds a command or widens an argument, a new dependency with
file system, network or FFI power, a decoder option, demuxer or protocol, a
new network destination, remote images or metadata, an updater, or a change
to CSP or capabilities. Such a pull request explains the threat and adds an
adversarial test before it can be merged.

## Adversarial tests

- IPC refuses unknown commands, window plugin calls, oversized and malformed
  arguments (browser and real-window suites).
- Local playback refuses UNC, device and alternate-stream paths, reparse
  points and swapped ancestors; the decoder refuses playlist, concat and
  external references and makes no connection (native suite).
- Radio refuses private, loopback, link-local, local-interface, mixed and
  changed DNS answers, redirect downgrade and loops, wrong certificates and
  host names, stalled and truncated responses (Rust radio suite).
- Scanning survives corrupt, truncated and oversized files and images,
  junction loops and escapes, access denied and a full disk without losing
  data (library suite).
