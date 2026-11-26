# ADR 0003: CLI/headless verification is mandatory

**Date:** 2026-11-22  
**Status:** Accepted

**Context:** AI agents must be able to develop, run and assess changes without remote desktop, manual window use or relying on unverified screenshots.

**Decision:** Pure Rust domain services, deterministic generated fixtures, mockable typed IPC, SQLite temp DB, libmpv null sink, local fixture HTTP server, Vitest, Playwright headless Chromium and Windows CI integration smoke. All normal tasks expose scriptable pass/fail commands and artifacts.

**Alternatives:** Manual-only GUI testing (not reproducible), WebView2-only UI tests (slower and environment-dependent), comprehensive browser mocks without native tests (misses native failures).

**Consequences:** More test harness engineering upfront, much higher autonomy and reliable regressions. Headless tests cannot certify actual speakers or all WebView2/SMTC behaviors, so release process still includes targeted runtime/hardware checks.
