# ADR 0002: libmpv + GPL-3.0-or-later

**Date:** 2026-11-22  
**Status:** Accepted. The pinned stable-source Windows candidate can be verified privately. **Public redistribution remains blocked.**

**Context:** ECHO needs reliable local codecs, internet radio, device handling and buffering. Reimplementing a media engine on CPAL/Symphonia would raise scope and failure risk.

**Decision:** One libmpv backend via narrowly scoped Rust FFI. ECHO original project code GPL-3.0-or-later. Use an audited compatible GPLv3-capable Windows libmpv build with all dependencies, notices and corresponding sources.

**Alternatives:** Rodio/Symphonia/CPAL (more PCM control but greater stream/device engineering), GStreamer (rich pipeline but integration/distribution complexity), bundled FFmpeg custom engine (significant maintenance).

**Consequences:** Mature playback but FFI ownership, native build/pinning, runtime DLL safety and legal distribution become core engineering requirements. Custom visualization DSP is harder; avoid making raw PCM a first-release blocker.

**Evidence:** The [native build](../NATIVE.md) describes the pinned Windows build. ECHO compiles Dolby Vision support out of libplacebo, which removes libdovi and its linked Rust runtime. The native source package records the remaining dependencies, receipts and notices. The [release licensing review](../LICENSING.md#release-approval-required) records the open redistribution decisions.

**Verification:** The [Rust radio transport](../RADIO.md) passes read-only streams to the same callback-only decoder used for files. [Native tests](../TESTING.md) cover decoding, rejection and cancellation. Physical output and device switching require separate verification. Record a reversal ADR before replacing the engine.
