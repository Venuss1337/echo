# Accessibility

Use WCAG 2.2 AA principles as an engineering goal, adapted to desktop software. This is a target, not a certification claim.

- Semantic HTML buttons, links, headings, tables, lists and dialogs where appropriate.
- Complete keyboard operation for playback controls, library filtering, playlists, queue reordering (include keyboard alternative to drag), menus and settings.
- Obvious `:focus-visible`, logical tab order, escape-to-close behavior and focus restoration after overlays.
- Screen-reader names for icon-only controls and playback progress, status announcements for errors and scanning; do not flood live regions with progress events.
- Text scale, zoom and Windows 125%–200% DPI must not clip core actions. No forced tiny text.
- Normal text target contrast ≥4.5:1, large text and nontext indicator target ≥3:1; audit against dynamic palettes.
- Honor reduced-motion; allow turning off artwork-derived effects/visualizers. Never use color or animation as the only status cue.
- Avoid single-gesture-only functionality; account for mouse, touchpad and keyboard.
- User data controls: clear destructive confirmations, undo/recovery opportunities, non-destructive media scans.

Automated tools (axe, Testing Library, Playwright) catch part of this. No automated suite alone proves accessibility.

## Automated coverage

 rendered text contrast under extreme covers and every placeholder tone (`contrast.spec.ts`); rail links keep their names; the drawer and dialogs hold focus, make what is behind them inert and give focus back; window buttons stay usable while a modal is open; keyboard focus in long lists never lands behind the dock; every dock control is reachable at every width; reduced motion starts no transition. The seek slider announces a time (`aria-valuetext`) and its hover preview is hidden from assistive technology. The native target over the maximize button is named "Maximize or restore"; the drawn button underneath stays in the tab order.

## Checked by a person

Screen readers (Narrator, NVDA), Windows contrast themes and displays at 125 % to 200 % scale have not yet been reviewed by a person for 1.0. The procedure is in the [manual acceptance checklist](TESTING.md#manual-acceptance); reports are welcome.
