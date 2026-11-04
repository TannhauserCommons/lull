# ADR-0080: lull makes each theme's picture itself, and the hub keeps it beside the theme

- Status: accepted
- Date: 2026-11-04

## Context

Settings › Appearance shows each design as a card. The five built-in designs ship a real picture (`assets/preview-{day,night}.webp`, 320×200, `"preview": true`). A listener's theme only has one if its author made it by hand, and none of the themes people actually make do (the owner's Halcyon and Relay, a friend's themes, anything saved in Theme Studio). Those cards fall back to a sketch: a few bars in the theme's four `swatches`.

The sketch can't look like the theme, however well it is drawn:
- **Four colours, day only.** `swatches` has no night set, so at night the card still shows the day colours (Halcyon's night is deep water; its card stays pale blue).
- **Most of a theme isn't colour.** Relay is concrete, cables, signal boxes and amber screens drawn from images; Halcyon is frosted glass over a lake. No token-driven sketch shows that, and a better sketch would still need the theme's CSS loaded on the settings page.

**Measured** (a prototype, on the fixture hub with copies of Halcyon and Relay): the page lull already is, drawn into a picture by the browser itself (the page's DOM and CSS, with fonts and images inlined, as an SVG `foreignObject`, onto a canvas):
- matches a real screenshot of Halcyon and Relay, day and night, by eye (glass, wallpaper, images, fonts, slots), once the root's attributes are carried to the SVG root (so `:root[data-theme="night"]` rules apply) and media queries are decided by the page being drawn;
- takes 0.15-0.4 s in Chromium and 0.4-0.9 s in WebKit on this workstation, after the page has loaded;
- works in Chromium (the Windows app, Android, Chrome) and in WebKit (the Linux app, Safari): the canvas stays readable, and both encode WebP.

## Decision

**lull makes the picture itself when the theme is made, and the hub keeps it.**
- **How a picture is drawn:** Home in that theme, in one off-screen frame at 1440×900 with motion reduced; once its fonts and covers are in, day and night (the modes the theme has) are drawn to 320×200 WebP, of the page's top left 1152×720 as the built-in designs' pictures are framed, and sent to the hub with the `rev` they show, then the frame goes. One theme at a time. Never a live frame per card.
- **When it is made, first:** the page that saves a theme in Theme Studio or imports one (a dropped file or "Import a theme") draws its pictures right away, in the background, for the `rev` the hub answered the save or import with. The save or import finishes first and never waits for it; a later save drops a draw still pending for the earlier one.
- **Why a fresh frame, there too:** Studio's two previews are drawn at the sheet's size, on whatever screen and size the author picked, with the draft as a separate sheet, so a picture taken from them wouldn't match one drawn anywhere else; an import has no page of the theme loaded at all. The off-screen frame loads the app files the page already has cached, so it costs about the same either way, and every picture is drawn the same way.
- **When it is made, as a fallback:** when Settings › Appearance lists a listener's theme with no picture of its own (`preview` false) and none made for its current `rev`. That covers the changes made with no lull page open (the author editing the folder on the hub, a new lull version: a theme's look depends on the lull it runs on) and a picture that didn't get made at creation (the page closed mid-draw, a 409 because the theme changed meanwhile). It starts once the screen has settled.
- **Keeping the device responsive:** the frame runs on the page's own thread. Copying styles, fonts and images yields as it goes. The steps no script can split (the frame's start, its change to night, the browser drawing the SVG) each wait until the page is idle and nothing was tapped, typed or scrolled for 1.5 s, so on a slow phone none starts while the listener is using the page. Measured at a 4× slower CPU, keys pressed every 0.4 s: none waits on a picture, except a key pressed as a step has just started, which waits for that step (0.3 s at most); the rest of the picture follows once the keys stop. Studio's own previews keep working throughout.
- A device that can't make one (no WebP encoding) leaves it to the next Appearance visit elsewhere. Until a picture exists, the card shows the sketch.

**A picture the author made still wins.** `preview: true` and the package's own files are unchanged, and lull never writes into a theme's package.

**The hub keeps made pictures beside the package, not in it** (`crates/themes`):
- `<data dir>/theme-pictures/<id>/<rev>-<mode>.webp`, outside `<data dir>/themes/`. So making one never fires `theme.changed` (devices using the theme don't reload it), and an exported `.lulltheme` never carries a picture of the listener's own library.
- `rev` names the theme's current files: the fingerprint the live-reload watcher already keeps (names, sizes, modification times) with the lull version. Any change to the theme gives a new `rev`.
- **API:**
  - `GET /api/themes`: each listener theme gains `rev` and `pictures`, the modes that have a picture made for that `rev` (`[]` when none). The manifest an import or a save answers with carries them too, so the page that made the change knows which `rev` to draw.
  - `GET /api/theme-pictures/{id}/{rev}-{mode}.webp`: the picture, cached as immutable (its name changes with the theme).
  - `PUT /api/theme-pictures/{id}/{rev}-{mode}.webp` (body: a WebP, 320×200, up to 256 KB): stores it. 409 when `rev` is no longer the theme's (it changed while the picture was being made, so a picture of old files is never kept); 400 for anything but a 320×200 WebP or a mode the theme doesn't have. Pictures of older revs of that theme are deleted.
  - Deleting a theme deletes its pictures.

**The card** shows, in order: the author's picture, the made one, the sketch; each for the mode the device shows, or for the one mode a single-mode theme has (today a day-only theme's card on a night device asks for a night picture that doesn't exist).

## Alternatives

- **A better sketch from the theme's tokens.** Light and needs no storage, but it would load every theme's CSS on the settings page and still can't show images, glass, slots or fonts: the cards would stay "a few coloured bars" for exactly the themes that differ most.
- **Write the made picture into the package** (`assets/preview-*.webp`, `"preview": true`, through the existing `PUT /api/themes/{id}/{path}`). No new endpoint, but each write fires `theme.changed` and reloads the theme on every device using it, an export would carry a picture of the listener's library to whoever gets the file, a made picture would be indistinguishable from the author's, and nothing could tell it went stale when the author edits the folder.
- **Keep made pictures on each device only** (IndexedDB). No hub change, but every device, the slowest phone included, would load the whole app a second time per theme to make its own copy.
- **A real screenshot on the hub** (a headless browser in the image). Exact, but a browser in the container is hundreds of MB for one picture per theme.
- **The browser's screen capture** (`getDisplayMedia`). Asks the listener's permission every time, and isn't in every webview.

## Consequences

- **A made picture shows the listener's own Home** (their recommendations and covers), drawn by whichever device made it. A touch device's picture shows what touch shows (no hover-only controls); the difference is small at 320×200.
- **A save or an import costs one extra off-screen draw** on the device that made it (about a second on a laptop, a few on a slow phone), in the background; a change made outside lull costs the same on the first device that opens Appearance after it. Once per theme change for the whole hub, not per device: another page showing Appearance when a theme changes waits 10 s and asks the hub again before drawing, so the page that made the change draws it alone.
- **The off-screen frame is a whole lull page,** as Theme Studio's previews are: while it lives it reports this device's state and the covers on its screen to the hub like any open page. A page drawn only to be looked at not acting as a device is a change of its own, for both.
- **`docs/themes.md`** says authors no longer need a picture (theirs still wins), and `api.md` lists the new fields and endpoints. The fixture hub models them, and lists the built-in designs first as the real hub does.
