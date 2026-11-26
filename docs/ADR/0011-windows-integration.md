# ADR 0011 — Windows media controls and single-window mini player

**Status:** Implemented. Windows shell and physical acceptance remain open.

The WinRT SMTC adapter owns its COM apartment and event registrations on a dedicated
thread. It projects confirmed playback state through a coalescing watch channel.
System actions and tray actions enter the bounded playback mailbox. The worker
resolves commands against the current source and validates seek generations.
The Windows timeline is present only for seekable local playback.
Album metadata and thumbnails are loaded on the media worker for the current
generation. The worker uses the read-only library, its bounded thumbnail,
or the validated station raster already in the radio cache. It discards
results after a source replacement and clears metadata at shutdown.

The tray registers fixed native actions. Hide to tray is explicit and close keeps
the existing exit behavior. The mini player reuses the main HWND, WebView,
playback subscriptions and compact presentation. It restores the prior native
placement when dismissed. The application remembers normal geometry and starts
in its normal mode. Existing main-window permissions cover two additional fixed
mini-player actions. There is no generic window creation or topmost command.
The explicit mini transition applies native z-order and checks the extended
style. A second launch uses the official Windows single-instance plugin
before setup, with the application identifier as its namespace. ECHO
discards forwarded arguments and restores the existing window in its mode.

The diagnostic snapshot is available through the native tray menu. Its schema is
an explicit allowlist of version, platform, schema and readiness fields. The
snapshot is saved locally to application data. It uses a read-only database
connection for the schema observation.
