# ADR-0007: Layouts reveal more as the window grows, and the listener can adjust them

- Status: accepted
- Date: 2026-11-01

## Context

The design package draws two sizes: phone (390 px) and desktop (1440 px). On a 2000 px window, lull stops at about 1,040 px of content, the side panel sits far off at the edge, and the middle is empty. The listener wants every window size to be used well. Stretching cards is not the answer: a bigger window should **reveal more**. They want Spotify-style resizable, collapsible sidebars, and their choices remembered per device (2026-11-01).

## Decision

**Size classes.** These come from the main area's own width (a CSS container query on `.main`), not the window's. Opening a panel or snapping lull to half the screen therefore works too.

| Class | Main area width |
|---|---|
| `compact` | below 640 px |
| `regular` | 640 to 1099 px |
| `wide` | 1100 to 1599 px |
| `xwide` | 1600 to 2199 px |
| `ultra` | 2200 px and up |

- `.app` carries `data-size="<class>"` and `data-height="short|tall"` (short is under 760 px), for scripts and themes.
- Containers are named `main`, `panel`, `section` and `tlist`.

**Reveal more, never enlarge.**
- Tiles, cards and rows keep their designed sizes.
- Grids use `auto-fill` and gain columns.
- Shelves show more tiles.
- Only prose has a maximum line length.

**What each class adds:**
- **Home:**
  - sections are modules in a grid: one column below `wide`, two at `wide`, three from `xwide`;
  - "Picked for you" shows more picks as its module widens;
  - a section with nothing to show collapses to a single line.
- **Library:**
  - grids gain columns;
  - song lists add columns as their own width grows: album (already at 760 px), then date added, plays, and format/bitrate.
- **Album and artist pages:** two columns from `wide`, with the header and details sticky on the left and the tracks on the right.
- **Search:** at `wide` and above, a toggle in the search header shows library and Soulseek results side by side.
- **Now Playing:** at `wide` and above, a button in the player bar opens a persistent Now Playing column with art, synced lyrics and transfer state.
- **The side panel** (Queue, Lyrics, Activity): from `xwide`, Queue and Lyrics can show as two panes at once.
- **Short windows** get tighter chrome; tall ones simply show more rows.

**Adjustable, remembered per device.**
- **Side panel and Now Playing column:**
  - resizable by dragging their edge, between set minimum and maximum widths;
  - also resizable with the keyboard: arrow keys on the focused handle, Home and End for the limits;
  - collapsible to an icon strip.
- **Home sections:** ~~reordered or hidden in an "Edit Home" mode~~. Removed 2026-11-02: it didn't work well and the listener didn't want it; Home's sections keep one order.
- **Where it's stored:** `localStorage` key `lull:layout` = `{ panelWidth, panelCollapsed, nowColumn, nowWidth, searchSplit, panelSplit }`. As with appearance, this is per device and not synced. Settings › Appearance › Layout has "Reset layout".

**One layout system, five designs.**
- Layout lives in the shared component and screen CSS.
- Each design adds its own touches in its `design` layer (decorations, how its shell uses the extra room).
- Themes use the same `data-size` and container names, documented in `docs/themes.md`.

**Performance.**
- Layout changes are pure CSS: container queries plus custom properties for the stored widths.
- Script runs only while a handle is dragged (`pointermove` throttled to animation frames) and when saving the layout.
- No layout reads in resize loops.

**Verification.** The visual matrix adds widths 768, 1024, 1280, 1920 and 2560, plus a 1366×768 short window, for every design and in both day and night.

## Alternatives

- **A fully modular panel system** (dock, split and rearrange anything). It's the most flexible, but it's much more work, and keeping it beautiful in five designs is hard. Adjustable panels and a reorderable Home give most of the value.
- **A different big-screen layout per design.** About five times the work and testing, for distinctiveness the shared system plus design touches already gives.
- **Wider max-width containers.** That's the stretched-cards result the listener rejected.

## Consequences

- **More states to test.** The matrix grows by six sizes.
- **Layout changes ride on CSS.** A design's CSS now lives alongside size-class rules, and the `design` layer must not undo them by accident. Themes document which hooks are safe.
- **Per-device layout state** means a phone and a desktop each keep their own arrangement, as intended.
