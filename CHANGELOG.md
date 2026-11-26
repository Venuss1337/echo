# Changelog

All notable changes to ECHO are documented here. ECHO follows
[Semantic Versioning](https://semver.org/).

## 1.0.0 — unreleased

The first public release.

### Library

- Index music folders on local drives, with incremental rescans, change
  watching and offline-drive handling that never removes your data.
- Browse albums, artists and tracks; instant full-text search; embedded
  artwork with a colour palette per album.
- Playlists, favorite tracks and albums, and listening history that survive
  moved, missing and removed files.

### Playback

- MP3, FLAC, WAV, Ogg Vorbis and AAC/M4A through libmpv 0.42.0 and FFmpeg
  9.1, built from source with networking and external file access compiled
  out.
- Queue with repeat and shuffle, restored at the next start.
- Clear errors when an output device disappears; Play resumes where it left.

### Radio

- Search the Radio Browser directory or add stations by URL; favorites and
  recent stations.
- Direct MP3 and AAC streams through a connector that only reaches public
  addresses, verifies TLS and reconnects after interruptions.

### Interface and Windows

- The Atmosphere 2.0 interface: Now Playing lit by the cover, a compact
  edition and an always-on-top mini player.
- Integrated title bar with Snap Layouts, media keys and the Windows media
  flyout, a tray menu for background playback, and a single running
  instance.
- Keyboard operation throughout, screen-reader labels, contrast checked
  against every cover, and reduced motion.

### Known limitations

- Windows 10 and 11 x64 only.
- The installer is not code-signed; SmartScreen may warn on first run.
- No gapless playback, crossfade, equalizer, ReplayGain or lyrics.
- Playlist stream formats (M3U, PLS) and HLS radio are not supported.
- Network shares cannot be added as music folders.
