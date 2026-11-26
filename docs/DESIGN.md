# Design: Atmosphere 2.0

Atmosphere 2.0 is ECHO's approved interface. Contributions fix defects and
extend it in its own terms; they do not restyle it. This document is the
reference for its rules.

## Floating surfaces

**Instruments float, music does not.** One continuous field, lit by the record, fills the window. Things you operate (navigation, the dock, the inspector, menus) are soft surfaces floating in it. Things you look at (prints, titles, listings) lie directly on it, uncaged. Prints stay square-cornered; instruments are round. That contrast is the identity.

## Why this direction

The earlier interface stacked two headers, trapped the atmosphere in one bordered box and repeated the same accents everywhere. Three compositions were compared:

- **A, open field:** the photograph lights the whole window and only instruments float. Chosen.
- **B, concentric plates:** every region, the page included, a rounded plate. It spends the atmosphere on gutters and cages the music again.
- **C, right-hand tower:** art, transport and queue in one column. It steals width from dense listings. Its type crossing the seam of the art became the compact edition.

Principles taken from references, not layouts: controls aligned to the columns they serve and a dock over the page (Daft Music, Cider); the page taking its colour from the record (Cider, Plexamp); artwork as an object with its own light, and type set beside it without a box (Sleeve); artwork as the interface (Longplay); one title line shared by navigation, search and window controls (Apple Music on macOS 26, Mornye). Astra and Aura Player were read for dense local-library conventions.

## Signatures

1. **The record stands in the dock.** Its sleeve is taller than the surface that holds it. Opening Now Playing takes it out and hangs it on the wall, and the dock is then only controls.
2. **The groove and the needle.** The whole track is one line along the dock's foot with a thin needle. The same needle marks the current page in the sidebar.
3. **Echoes.** Two turned impressions of the same photograph behind every print. Every record opens a little when you reach for it. Three large ellipses from the mark lie in the field.
4. **Spines.** A playlist is the standing spines of its records, so it is never mistaken for an album.
5. **The exhibition.** Now Playing is one work on a wall lit by its own cover, with a museum label. The title hangs over the echoes; small type stands clear of them. Neighbouring records in the queue hang further along, cropped by the window, and can be played.

## The field

`shell/Field.tsx`. The room is lit by what is playing. A page about one record (album, artist, playlist) lends its own light while it is open; the dock and the queue keep the light of what is heard. Layers: base and two glows from the palette, the photograph, a wash that keeps type on a dark ground, the ellipses, a static grain against banding, and a vignette under the caption.

| Exposure | Pages | Photograph |
|---|---|---|
| `lit` | Home, artist, Now Playing, compact, empty states | full, washed from the reading side |
| `read` | album, playlist | shaded rather than washed, and kept to the header, so listings have a dark ground |
| `low` | every listing, and any page once scrolled 150 px | colour only |

## Materials

Derived in CSS from `--ink`, `--glow` and `--accent`, wherever a light is set. The Rust extractor still guarantees those three. No component blurs what is behind it.

| Token | Use |
|---|---|
| `--base` | floor of the field |
| `--smoke` | navigation, 82% over the field |
| `.plane` | quiet ground under dense listings; absent on lit pages |
| `--panel` | contextual inspector |
| `--raised` | dock, opaque, the most lifted surface |
| `--over` | menus, dialogs, drawer, opaque |
| `--hover`, `--press`, `--selected`, `--well` | states and inset fields |
| `--lift-1..3` | one light from above: a top edge, then shadow. No borders. |

Text tokens (`--text`, `--muted`, `--faint`) are fixed. The accent follows the record, so play buttons, needles and current rows change with the music.

## Shape, space, type

Window gap 10 px, title band 54 px, panels 18 px, dock 24 px, rows 8 px, prints 3 px. Sidebar 222 px, rail 66 px, inspector 316 px. Rows 56 px (46 px on an album, where the artwork column is omitted). Instrument Serif carries display, record titles and section names (the library's tabs are its headings); DM Sans carries everything operated. No tracked capitals.

Type and layout inside the page answer to **the canvas** through container queries, because it shares the window with other surfaces. The frame answers to the window: full sidebar, rail below 1020 px (or 1240 px while the inspector is open), drawer below 680 px. The inspector floats over the page, above the dock, below 900 px.

## The dock

Three arrangements by its own width: one row with a slider; the volume opening upwards below 830 px; three decks below 660 px. Shuffle, previous, play, next, repeat, seek, mute, queue, compact and the overflow menu are visible at every width. Stop and Open audio files moved to that menu. The page keeps `--dock-clear` below its last row and as scroll padding, so nothing ends or is focused behind it.

## Motion

`shell/motion.ts` applies a change synchronously and animates only its presentation, one choreography per change, keyed by `data-motion`:

- **inspector, sidebar:** room is made, then taken. Surfaces hold; only the reflowing page fades.
- **expand:** the record travels from the dock to the wall.
- **compact:** the dock unfolds into the card.
- **record:** a cover travels from its shelf to its page.
- Menus unfold from the control or pointer that asked. Dialogs settle. The next work is hung from the side it waited on. New light cross-exposes once over a second.

Nothing loops. Idle scenes run no animation and mutate no DOM. Interpolation stops while hidden or minimized. Reduced motion removes all of it and starts no transition.

## Title bar

See [Windows integration](WINDOWS.md) and [ADR 0009](ADR/0009-integrated-window-chrome.md). Sidebar toggle, back, forward and the one search field; brand in the sidebar head; caption buttons flush in the corner at the full height of the band, with Windows' own metrics and close colour. It has no background of its own.

## Compact edition

A full-bleed print that develops into ink, the title across the seam, the groove below it. It is the window in a small window, a floating card in a large one, and turns on its side in a wide, short one. The always-on-top mini player is this edition in the same window, at 420 px wide.

## Acceptance

`tests/browser/contrast.spec.ts` measures every visible run of text against the pixels behind it, under white, black, yellow, red, blue and cyan covers and all placeholder tones, at three widths: 4.5:1, or 3:1 for large type. New surfaces must pass it.

## Radio

Radio opens directly on the continuous field. The station directory uses a
single featured sleeve followed by dense discovery rows. Search and filters
stay in place when the feature gives way to search results. The floating
inspector is where station details and manual editing live. Live audio lends
its station's sleeve and palette to the existing exhibition, dock and compact
edition. In live playback, a static LIVE line takes the place of the seek
groove. The same controls lead back to the local queue. Baselines live
alongside `tests/browser/radio.spec.ts`.


## Assets

Fonts are bundled (`public/fonts`, SIL Open Font License, licenses beside
them); nothing is fetched at runtime. Artwork comes only from the user's own
files and stations. Covers without art use the ECHO imprint in one of six
stable tones.

## Anti-patterns

No blur behind surfaces, no continuous or looping animation, no bordered
cards around music, no tracked capitals, no colour taken from artwork without
the contrast guarantee, no generic dashboard components.
