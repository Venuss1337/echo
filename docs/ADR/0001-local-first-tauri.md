# ADR 0001: Tauri 2, local-first Rust application

**Date:** 2026-11-22  
**Status:** Accepted for implementation; environment verification pending

**Context:** ECHO needs a polished native Windows app with React UI, native audio, local database and no cloud/backend bill.

**Decision:** Tauri 2 + WebView2 with Rust domain services and local SQLite. Typed commands/events, no embedded HTTP API. React is purely the view layer. Windows initially.

**Alternatives:** Electron (more runtime overhead, straightforward web ecosystem), fully native Rust GUI (less React reuse, more custom rendering), browser web app (weaker native capabilities).

**Consequences:** Smaller desktop shell with solid Rust integration, but WebView2 runtime and Windows integration require explicit testing. Tight IPC/security boundaries required. Separate Astro marketing site remains static.

**Verification needed:** official stable toolchain bootstrap, clean Windows build, CSP/capabilities tests, packaged startup.
