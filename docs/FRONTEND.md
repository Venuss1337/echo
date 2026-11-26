# Frontend

## Source-of-truth separation

Rust owns domain state; React renders it. A provider holds the mirrored, Rust-confirmed player snapshot, and local presentation state (sidebar mode, open inspector pane) lives in components. TanStack Query caches paginated library and radio queries and is invalidated by revisions from the host. Neither is a second copy of the library.

## Structure

`main.tsx` owns the providers, routing, the error boundary and, outside that boundary, the caption buttons. `App.tsx` owns the shell: routes, shortcuts, exposure, the open inspector pane and the sidebar mode. `shell/` holds what floats around the page: `Field`, `TitleBar` (with the one search field, which owns the Search page's query), `Sidebar`, `Inspector`, `chrome` (the window binding) and `motion`. `Dock`, `Queue`, `NowPlaying`, `Compact` and `transport` are the player's views over one confirmed state.

`styles/` is one ordered system: tokens, base, controls, shell, dock, inspector, library, pages, exhibition, compact, motion. Light tokens live on the document element; a surface that carries another light sets them again on itself, and derived materials are re-declared there. Pages use container queries on `canvas`, the dock on `dock`.

A new inspector pane is a name in `InspectorPane`, a component that renders `Inspector`, and a control that opens it. The sidebar preference is the only value in `localStorage`; losing it is harmless.

## Player UI

The dock persists across route navigation and reads Rust-confirmed state. Now Playing expands from the player and can change backdrop palette without remounting a second audio controller. Player progress uses local animation of throttled snapshots, not a timer that assumes music actually plays.

## Data-intensive UI

Use virtualization for tracks/search results; stable row IDs, keyboard navigation and screen-reader semantics. Queries use paging, filters, explicit sorting and a cancellable request path. Avoid rendering 20k track components or passing 100MB metadata blobs into React.

## Component standards

- Prefer semantic native controls. Menus, dialogs and the drawer are authored in `ui.tsx` with tested focus behaviour.
- Author ECHO design components with tokens, not a default copied dashboard.
- Meaningful states: default, hover, active, focus-visible, pressed, disabled, loading, success/error, skeleton and empty.
- Global shortcuts require discoverability and must not fire while the user types into inputs or dialogs.
- State derivations are memoized only when profiling justifies it. Avoid premature memoization and sprawling context providers.
- Use type-safe exhaustive switches on playback states; don't let an unknown state silently display Playing.

Generated types come from `src/generated/contracts.ts`; boundary validators (`status.ts`, `library/api.ts`, `radio/api.ts`) check incoming IPC before it reaches components.

## Testability

Use an injectable typed IPC client with contract-accurate mock implementations. Components should render in Vitest/Testing Library and Playwright Chromium without Tauri runtime. Test accessible roles and behavior, not CSS classes or internal functions. Keep design snapshots deterministic by mocking time, fonts, artwork and animation. WebView2 parity is checked separately by the private-desktop harness (`docs/TESTING.md`).
