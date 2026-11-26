# Internet radio

ECHO plays direct MP3 and AAC internet radio. Rust owns discovery, saved
stations, artwork and every connection to a broadcaster; the decoder only
ever receives bytes. The connector's design is recorded in
[ADR 0010](ADR/0010-radio-connector.md).

## Discovery

[Radio Browser](https://api.radio-browser.info/) supplies the directory.

- Search by name, country, language or tag; pages of 24 stations sorted by
  popularity, name or recent activity. The page debounces typing by 320 ms
  and cancels superseded searches.
- Mirrors come from DNS SRV discovery, are tried in random order with at most
  three attempts per search, and are remembered for five minutes.
- Responses are limited to 768 KiB and validated row by row. Browsing never
  probes an audio stream; a result's online status is Radio Browser's.
- Pages stay fresh for ten minutes and can be served stale for 24 hours
  during an outage. SQLite keeps at most 96 pages and 4,096 directory station
  references. Favorites and manual stations are never evicted.
- Requests identify ECHO by version. An explicit Play of a directory station
  reports a click to Radio Browser, as its API asks; resume, reconnect and
  manual stations never do, and no user identifier is sent.

## Stations and the renderer

The page sees stable station IDs and display metadata, never stream or
artwork URLs. The only URL input is the manual station editor: saving checks
syntax and the literal host without contacting the station. Every playback
attempt authorizes the URL again at connection time.

## The connector

Each hop of a stream, directory or artwork request goes through
`radio/net.rs`:

1. Parse the URL: HTTP or HTTPS, no credentials or fragment, a bounded
   length, and a dotted public host name (single-label names, `.local`,
   `.localhost` and `home.arpa` are refused).
2. Resolve the host, and reject the whole answer if **any** address is not
   public. IPv4-mapped IPv6 is classified as IPv4 first. IPv4 excludes whole
   special-purpose blocks: `0/8`, `10/8`, `127/8`, `100.64/10`, `169.254/16`,
   `172.16/12`, `192.0.0/24`, `192.0.2/24`, `192.88.99/24`, `192.168/16`,
   `198.18/15`, `198.51.100/24`, `203.0.113/24` and everything from `224/4`
   up. IPv6 must be global unicast (`2000::/3`) outside `2001::/23`,
   `2001:db8::/32`, `2002::/16` and `3fff::/16`. Every address assigned to a
   local interface, including global ones, is refused as well.
3. Connect to one of those checked addresses only. Proxies, automatic
   redirects and retries in the HTTP client are disabled, so nothing can
   resolve or connect a second time behind the check.
4. Verify TLS offline with Rustls against bundled Mozilla roots and the
   original host name; no intermediate fetching, no revocation requests, no
   enterprise trust overrides. A broadcaster must send a complete chain.
5. Follow at most four redirects, each from step 1, and never from HTTPS to
   HTTP.

DNS, connect, complete headers and body idle time each have five-second
deadlines; waiting for room in the decoder queue does not count against the
body timer. Headers are limited to 32 fields.

## Decoding

The stream is served to libmpv through a one-use `echo-radio://` callback
for a single decoder generation. ICY metadata is stripped before the decoder
sees it; the sanitized stream title (including artist text when the
broadcaster sends it) is shown, otherwise the station name. The transport
buffers four 16 KiB chunks plus the chunk being read; the demuxer keeps at
most 512 KiB of forward data and no back cache. The first native probe has a
15-second deadline. The callback offers no seek or size, and is cancelled
before the decoder is destroyed. Playlist formats (M3U, PLS), HLS and legacy
ICY status lines are not supported and fail clearly.

## Artwork

Only the featured, inspected or playing station requests artwork. The
connector fetches at most 768 KiB, the bytes must sniff as JPEG or PNG within
two megapixels, and the image is re-encoded to a JPEG of at most 320 px with
a palette. The disk cache holds 96 entries of at most 128 KiB, refreshed after
24 hours, with failures retried after ten minutes.

## Playback behaviour

- A station is a separate source in the same player: the local queue and its
  position stay put, and playing a queue entry returns to local audio.
- Live radio has no seek, next, previous, repeat or shuffle. Volume and mute
  are shared.
- Pause disconnects; resume starts a fresh connection at the live edge.
- After an interruption ECHO retries three times at 0.8, 1.6 and 3.2 seconds
  plus up to 25 % jitter. Stop, pause, replacement and exit cancel the retry,
  the socket and the decoder. Security and format rejections are final.
- The newest 100 stations confirmed playing are kept as recent. Startup never
  connects to a station.

## Availability

Station playback requires the packaged, reviewed decoder. Development builds
without it show radio as unavailable.

## Tests

- `cargo test --locked --manifest-path src-tauri/Cargo.toml --lib radio::`
  runs the connector against loopback fixtures with a synthetic TLS chain:
  address pinning, mixed and changed DNS answers, redirect depth, loops and
  downgrade, wrong certificate and host name, truncated and stalled bodies,
  backpressure, cancellation, malicious directory and ICY data. Loopback and
  test roots are allowed only inside the unit-test executable.
- `native/windows-x64/test-live.ps1` plays MP3 and AAC through the real
  decoder: restart, source switching, pause and resume, reconnect and
  exhaustion, a misleading playlist MIME type and stalled shutdown.
- No automated test depends on a public station. The private-desktop harness
  can optionally tune this repository's own fixture files over public HTTPS
  (`-FixtureRadio`), verified byte for byte against the checkout.
