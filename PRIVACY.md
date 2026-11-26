# Privacy

ECHO is local-first. It has no account, no telemetry, no analytics, no crash
reporting service and no server of its own.

## What stays on your computer

Your library index, playlists, favorites, listening history, queue,
settings, artwork cache, cached radio directory pages and diagnostics are
stored in ECHO's application data folder
(`%LOCALAPPDATA%\org.echo.player`). ECHO never modifies or uploads your
music files. Uninstalling offers to delete this data; otherwise it is kept.

## What goes over the network

Local playback works offline. ECHO connects to the network only for radio:

- **Radio Browser** (when you search the directory, or play a station from
  it): your IP address, the search, and ECHO's version. Playing a directory
  station reports a click for that station, as the directory requests; no
  identifier of you or your device is sent.
- **The station's broadcaster** (when you play a station, and for its
  artwork): your IP address, ordinary HTTP request information and ECHO's version.

These services have their own privacy practices. ECHO cannot make radio
listening anonymous.

## Diagnostics

*Export diagnostics* in the tray writes a small file to the application data
folder: ECHO's version, Windows platform information, the database schema
version and readiness flags. It contains no paths, track names, station URLs
or logs, and it is only shared if you choose to attach it somewhere.
