# ADR 0010 — Radio uses offline certificate verification

**Status:** Accepted for the M5 implementation.

The Rust connector checks every resolved address before any HTTP connection. A
system TLS chain builder can retrieve missing intermediates outside that
connector. An earlier Schannel configuration limited revocation requests to its
cache, but did not explicitly disable intermediate retrieval.

The pinned reqwest client uses a preconfigured Rustls client with ring and the
Mozilla trust anchors packaged by webpki-roots. WebPKI validates the supplied
chain, server name and certificate validity without additional network
requests. It does not perform online revocation checks. Enterprise trust
overrides and ambient proxies are excluded. A broadcaster must supply a
complete publicly trusted chain. Dependency maintenance must keep the bundled
trust roots current. The trust-anchor data is CDLA-Permissive-2.0.

The native decoder remains the reviewed callback-only M2 artifact. Radio,
directory and artwork requests share the pinned connector. Only deterministic
unit-test processes can supply fixture addresses or add a test root.
