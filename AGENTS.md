# ECHO — instructions for coding agents

ECHO is a local-first music and internet radio player for Windows: Tauri 2,
a Rust host that owns playback, storage and networking, and a React
renderer that only presents. These instructions apply to every change.
Subsystem rules live in `AGENTS.md` files next to the code they govern.

## Read before changing

- `docs/ARCHITECTURE.md` for ownership and module boundaries.
- The one or two domain documents your change touches:
  playback `docs/PLAYBACK.md`, IPC `docs/CONTRACTS.md`, library and SQLite
  `docs/LIBRARY.md`, radio `docs/RADIO.md`, window and shell
  `docs/WINDOWS.md`, interface `docs/DESIGN.md` and `docs/FRONTEND.md`,
  decoder build `docs/NATIVE.md`, release `docs/RELEASE.md`.
- `docs/SECURITY.md` for anything that touches files, URLs, IPC or native code.

Do not read every document for a small change.

## Boundaries that are not negotiable

- **Rust owns behaviour.** Playback, queue, filesystem access, SQLite,
  radio networking, artwork decoding and Windows integration live in
  `src-tauri`. The renderer sends typed intents and renders confirmed state.
- **No new renderer power.** Do not grant Tauri plugin permissions, file
  system or shell access, or accept paths, URLs or decoder options from the
  page. A new command validates every argument in Rust and is added to
  `src-tauri/capabilities/default.json` explicitly.
- **One playback engine.** The reviewed libmpv build behind
  `playback/decoder.rs`. No second audio stack, no `<audio>` element.
- **Contracts are generated.** Wire types are Rust structs in
  `src-tauri/src/contract.rs`; `bun run gen:contracts` writes
  `src/generated/contracts.ts`. Never edit the generated file by hand.
- **User data is never sacrificed.** No migration, scan or cleanup may delete
  playlists, favorites, history or music files. Migrations are forward-only
  and tested against previous schemas.
- **The design is fixed.** Atmosphere 2.0 (`docs/DESIGN.md`) is the approved
  interface. Fix defects within it; do not restyle, re-theme or recompose
  views. Visual baselines change only for a demonstrated defect.

## Commands

PowerShell 7 on Windows x64, from the repository root:

```powershell
bun install --frozen-lockfile
bun run verify          # build, lint, format, types, contracts, Rust and browser tests
bun run test:security   # advisories, licenses, registry sources
bun run tauri dev       # the desktop app (audio needs a staged decoder, see docs/NATIVE.md)
```

Narrower loops: `bun run check`, `bun run test:unit`, `bun run test:db`,
`bunx playwright test <spec>`,
`cargo test --locked --manifest-path src-tauri/Cargo.toml <filter>`.
Native, installer and real-window suites are in `docs/TESTING.md`.

## Definition of done

1. The behaviour, including its failure cases, is covered by a test at the
   lowest level that can observe it.
2. `bun run verify` passes, or the parts you ran are named with their real
   results. Never report a command you did not run as passing.
3. No test, lint rule, security check or baseline is weakened, skipped or
   deleted to get green. If a test is wrong, show why and fix it with the
   behaviour.
4. Comments explain why: invariants, ownership, `unsafe` (`// SAFETY:`),
   races and security reasoning. No narration of obvious code.
5. Documentation that describes the changed behaviour is updated in the same
   change. Add an ADR in `docs/ADR/` for a decision a future contributor
   would otherwise reopen.
6. New dependencies are justified, pinned by the lockfiles and pass the
   license policy in `deny.toml`; shipped assets carry their license.

## Working style

- One coherent change per pull request. No drive-by refactors, renames or
  reformatting of unrelated code.
- Prefer the platform and existing modules over new crates, packages or
  abstractions.
- Tests are deterministic and run without a mouse, a visible window, public
  radio stations or personal music. Real-window checks use the private
  desktop harness and never take the user's pointer or focus.
- Commit messages are plain, imperative and professional. Do not add
  generated-by or co-author trailers for tools.
