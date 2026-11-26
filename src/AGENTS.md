# Renderer and Atmosphere 2.0

Read `docs/DESIGN.md`, `docs/FRONTEND.md` and `docs/ACCESSIBILITY.md` first.

- The renderer presents Rust-confirmed state. Do not simulate playback,
  keep a second queue, or hold library collections in component state;
  page and virtualize through the typed commands.
- Atmosphere 2.0 is the approved design. Keep its composition, type,
  colour, materials, motion and the compact edition. Work within
  `src/styles` tokens; do not introduce another styling system, component
  kit or theme.
- Every interactive element has a keyboard path, a visible focus state and
  an accessible name. Dialogs and drawers trap and restore focus. Reduced
  motion starts no transition.
- Nothing animates or polls while idle, hidden or minimized. Measure before
  adding layers, filters or continuous effects (`docs/PERFORMANCE.md`).
- `generated/contracts.ts` is produced by Rust. Validate incoming IPC at the
  boundary as the existing modules do.
- Tests: `bun run test:unit`, `bunx playwright test`. Baselines change only
  for a reviewed, intentional fix: `bunx playwright test --update-snapshots`
  then inspect every diff.
