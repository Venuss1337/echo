# ADR 0006 — Give the local decoder bytes from an approved file handle

Status: Accepted for the private Windows x64 local candidate.

M2 requires an isolation boundary for playback of user-selected files. The feasibility DLL has native HTTP and file protocols. Rejecting remote-looking filenames and setting mpv options would still leave those openers in the decoder.

Build the pinned mpv 0.42.0 / FFmpeg 9.1 sources with the existing UCRT64 recipe's `local` profile. Compile FFmpeg with networking and every URL protocol disabled. Restrict its allowed demuxers to AAC, FLAC, MOV, MP3, Ogg and WAV. Patch mpv's stream registry to the read-only callback handler only and its demux registry to lavf only. Keep configuration, reference loading, auxiliary media, display and scripting disabled. Keep the earlier feasibility profile and its radio experiments available for testing. M5 uses this same profile to decode bytes from its approved Rust connector. See [ADR 0010](0010-radio-connector.md).

The Windows host selects and opens ordinary files. It validates the drive and resolves its local volume GUID, rejects UNC/device/alternate-stream paths, and walks each path component without following reparse points. Checked ancestors stay open without write/delete sharing until the final read-only file is held. mpv sees a fixed opaque URI for a single-use registration. Every generation has its own core and source handle. The Windows loader accepts only the entire reviewed DLL set for desktop playback, and the adapter verifies the active core advertises no built-in protocols.

This removes decoder network and secondary file openers from the intended build. It does not isolate arbitrary native code execution caused by a decoder vulnerability. The file chooser itself has normal Windows Shell navigation, and application source acceptance still rejects network destinations. Ordinary playback uses WASAPI with null fallback disabled. Read-only null output is explicit in tests.

Two Windows build roots reproduced the same local DLL. The staged service verified real playback events, seeking, stopping and EOF using generated audio. The adversarial suite supplied M3U, PLS and concat references and checked that the loopback listener received no connection. A negative control rejected the older feasibility DLL before source load. See the [native build](../NATIVE.md), [tests](../TESTING.md), and [redistribution requirements](../LICENSING.md).
