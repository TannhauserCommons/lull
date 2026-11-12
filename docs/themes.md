# Writing a lull theme

A theme restyles lull on every device that picks it. It is plain CSS, plus optional HTML templates for a few pieces of markup and the fonts and images they use. The five designs lull ships with (Lull, Skydeck, Starlight, Petal, Glasshouse) are themes in exactly this format, in `web/designs/<name>/`: read one when you want to see how far a theme can go.

Ten minutes from here you can write one. The decision behind it is ADR-0004.

(Editing a built-in design? Each also ships as one generated script, `web/designs/<name>.js`, so a page loads it in a single request. After changing its `theme.json` or `slots/`, run `LULL_WRITE_PACKAGES=1 cargo test -p lull-themes`; the test fails while it is stale.)

## 1. What a theme is

A folder, or a `.lulltheme` file (a zip of that folder):

| File | Required | What |
|---|---|---|
| `theme.json` | yes | What the theme is (below) |
| `theme.css` | yes | Any CSS; may be empty |
| `slots/<slot>.html` | no | Your markup for a slot (section 6) |
| `assets/` | no | Fonts, images and font licence texts, used from `theme.css` as `url("assets/...")` |
| `pack.json`, `sounds/`, `pack.css`, `slots/egg.*.html` | no | The theme's sounds and easter eggs (section 13) |

```json
{
  "id": "rainy-day",
  "name": "Rainy day",
  "author": "you",
  "api": 1,
  "extends": "lull",
  "shell": "topnav",
  "swatches": ["#dfe6ee", "#f6f8fb", "#3b6e9c", "#1d2733"],
  "modes": ["day", "night"],
  "description": "Lull's windows in grey-blue."
}
```

- `id`: 1-40 lowercase letters, digits and dashes; not a built-in design's name.
- `name`: the card's title. `author` (optional): your name. A theme begun from a copy of a built-in design still says `lull` until you change it, and Theme Studio leaves it for you to sign.
- `api`: the theme API version you wrote for (section 9). Today it is `1`.
- `extends`: a built-in design (`lull`, `skydeck`, `starlight`, `petal`, `glasshouse`) whose CSS and slots load first, or `null` to start from the base look alone.
- `shell`: the layout on desktop and tablet: `rail` (a side menu), `topnav` (pills in the header) or `dock`. Phones always use the tab bar.
- `swatches`: page, surface, accent and ink colours (`#rrggbb`) for the sketch on the picker card in Settings › Appearance, shown until the card has its picture.
- `modes`: `["day", "night"]`, or just one. A one-mode theme ignores the day/night switch.
- Optional: `description`, `preview` (true when you made the picker card's pictures yourself, `assets/preview-day.webp` and `-night.webp`, 320×200; without them lull makes them from your theme, ADR-0080), `coverVariety` (section 5), `window` (a see-through window in the desktop app, section 12). The hub fills in `slots` itself.

**How long the words may be.** As long as they need to be: each screen shows what fits. The picker card shows the name and the first two lines of the description, and clips the rest with an ellipsis; Your themes, Theme Studio and screen readers have all of it. Theme Studio tells you when the card will clip the description; that is a hint, never a reason not to save. The hub refuses only text no theme needs, a guard against a manifest made to bloat every list it is in: a name or author over 200 characters, a description over 2,000. It says which and by how much (`theme.json: description is 2001 characters, at most 2000`).

**Before you share one,** check it as the hub's import does:

```sh
cargo run -p lull-themes --bin lull-theme-check -- my-theme.lulltheme my-theme/
```

It prints `ok` or the reason the import would give, for each `.lulltheme` or package folder; a script that packs themes runs it on what it made.

**Installing one:** drop the `.lulltheme` on Settings › Appearance › Your themes, or make it in Theme Studio there. Both store it on your hub in `<data dir>/themes/<id>/`, so every device can pick it. You can also edit the files in that folder directly: open devices reload the theme within a second.

**Its card in Settings › Appearance** shows a picture of Home in your theme, day and night, which lull draws itself right after an import or a Theme Studio save, and on the next visit to Appearance after any other change (an edit in the folder, a new lull). The hub keeps it beside the theme, never in its package. A picture you ship yourself (`preview`) is shown instead.

**Safe mode:** add `?theme=safe` to the address (`https://your-hub/?theme=safe`) to boot the default design, whatever this device had picked. The device keeps it from then on, so a theme that broke the app stays away until you pick it again. Deleting a theme moves every device using it back to the default too, including one that was off at the time (at its next start).

## 2. Your first theme: tokens only

lull draws everything from custom properties ("tokens"). A theme that sets a few of them is complete:

```css
/* theme.css */
:root {
  --f-ui: "Atkinson Hyperlegible", system-ui, sans-serif;
  --r-card: 8px;
  --r-panel: 8px;
}
:root[data-theme="day"] {
  --sky: #dfe6ee; --panel: #f6f8fb; --link: #3b6e9c; --accent-ui: #3b6e9c;
}
:root[data-theme="night"] {
  --sky: #1d2733; --panel: #26323f; --link: #9cc3e6; --accent-ui: #9cc3e6;
  --fg-sky: #e8eef4; --fg-panel: #e8eef4;
}
```

Theme Studio writes exactly this kind of block for you (between `/* tokens: written by Theme Studio */` and `/* end of tokens */`) from its colour pickers and sliders, and warns when a text colour doesn't stand out enough from its background, and when a loop names no rate, redraws at every step or runs under glass (section 5, Drawing cheaply).

## 3. Why you never need `!important`

lull's CSS is in cascade layers, in this order: `reset`, `tokens`, `base`, `components`, `screens`, `design`, `user`. A later layer wins over an earlier one whatever the selectors, and your `theme.css` is the last one, `user`. So `.card { border-radius: 0 }` beats any design's `:root[data-design="lull"] .app .main .card` rule. Inside your own file the usual rules apply (specificity, then order). You don't write `@layer` yourself; lull puts your file in `user`.

An `!important` in your theme can't beat one of lull's: an important declaration in an earlier layer wins over one in a later layer.

**What lull holds fixed, and why.** lull's own CSS (`web/*.css`, `web/screens/*.css`, `web/desktop/`, not the designs in `web/designs/`, which are themes) uses `!important` only where something must hold whatever a design or theme draws (ADR-0101): a part that gave way leaves its row, a strip hides its panel's contents, reduced motion stops movement, `[hidden]` hides, the desktop app keeps its window's buttons clear. These are the only places your CSS can't change; style around them. `tests/web/important.test.mjs` (in `mise run check`) fails a new one that isn't listed here, and a listed one that is gone.

| File | Rule | Holds | Why |
|---|---|---|---|
| `base.css` | `[hidden]` | `display` | What lull hides with `hidden` (a state tab while all is well, a pane not in use) stays hidden, whatever `display` a design gives it. |
| `base.css` | `:root[data-window]` | `background`, `color-scheme` | A see-through window (section 12): the wallpaper steps aside for `--window-backdrop`, so the desktop shows through; Chromium paints a root opaque when its colour scheme differs from the webview's. |
| `base.css` | `:root[data-window] body` | `background` | The same: the page's body lets the desktop through. |
| `base.css` | `:root[data-window] .sky-specks` | `display` | The same: the wallpaper's specks step aside with it. |
| `base.css` | `:root[data-window-preview]` | `background` | The browser's preview of a see-through window (`#/home?window=blur`) shows the worst desktop there is, black and white, under the glass. |
| `base.css` | `:root[data-window-preview] body` | `background` | The same preview: the page shows `--window-backdrop` over it. |
| `base.css` | `.vh` | `position` | Words for screen readers only take no room and show nowhere, whatever a design sets on an element they share. |
| `base.css` | `:root:not([data-motion="full"]) *, :root:not([data-motion="full"]) *::before, :root:not([data-motion="full"]) *::after` | `animation`, `scroll-behavior`, `transition-property`, `transition-duration` | The system's reduced motion stops movement: loops stop and transitions keep only their fades, unless the listener chose full motion. |
| `base.css` | `:root[data-motion="reduce"] *, :root[data-motion="reduce"] *::before, :root[data-motion="reduce"] *::after` | `animation`, `scroll-behavior`, `transition-property`, `transition-duration` | The listener's own reduced motion, the same. |
| `components.css` | `.header :is(.kbd, .tray__label-long, .tray--full, .tray__word, [data-act="theme"])[data-fit-out], .header .nav-pill[data-fit-out] .nav-pill__label` | `display` | A part of the header that gave way leaves its row (section 5, Never patch a width band), so the row fits. |
| `components.css` | `.header .tray--full[data-fit-out] ~ .tray--compact` | `display` | When the full status tray gives way, the compact one takes its place: the services' state stays one click away. |
| `components.css` | `.header .header__search[data-fit-out]` | `min-width` | The search that gave way gives up its room (down to 96 px), whatever least width a design gives the field, so the header fits. |
| `components.css` | `.app[data-panel="strip"] .ctx > :not(.ctx__strip)` | `display` | The side panel collapsed to its strip hides its contents. |
| `components.css` | `.app:is([data-nowcol="open"], [data-nowcol="strip"]) .main` | `margin-right` | The main area makes room for the Now column at its dragged width, so the column never covers the page. |
| `components.css` | `.app[data-nowcol="strip"] .nowcol > :not(.nowcol__strip)` | `display` | The Now column collapsed to its strip hides its contents. |
| `components.css` | `.pbar :is(.btn-icon, .seek:not(.vol > .seek), .pbar__from)[data-fit-out], .pbar__badges > [data-fit-out]:not(.pbar__private)` | `display` | A part of the player bar that gave way leaves its row; what it did is in the "…" menu. |
| `components.css` | `.pbar .vol > .seek[data-fit-out]` | `position` | The volume slider that gave way leaves the row and opens over the volume button. |
| `components.css` | `.pbar__private[data-fit-out] .pbar__private-label` | `display` | Private that gave way keeps its eye; its word leaves the row. |
| `components.css` | `.pbar__acts[data-fit-out]` | `display` | Like and Save that gave way leave the row; both are in the "…" menu. |
| `components.css` | `.mini [data-fit-out]` | `display` | A part of the mini player that gave way leaves its row, never over a button: the state chip, and the key for another device (the song's "…" menu, on Now Playing, has what it did). |
| `components.css` | `:root.is-hidden *` | `animation-play-state` | Loops pause while the page is hidden: nothing is drawn for a page nobody sees. |
| `desktop/desktop.css` | `:root[data-titlebar="win"] .app:not([data-window-controls="own"]) .header` | `padding-right` | The desktop app on Windows draws its minimise, maximise and close buttons over the page's top right corner (138 × 32 px): no header key goes under them, unless the theme lays out round them itself (`--layout-window-controls: own`). |
| `desktop/desktop.css` | `:root[data-titlebar="win"][data-shell="topnav"] .app:not([data-window-controls="own"]) .header, :root[data-titlebar="win"][data-shell="dock"] .app:not([data-window-controls="own"]) .header` | `padding-right`, `margin-right` | The same, for a header drawn as a strip: the strip stops short of the buttons. |
| `desktop/desktop.css` | `:root[data-titlebar="win"] #app:not([data-window-controls="own"])` | `padding-top` | The same, from 640 to 819 px, where a header can't spare the room: the buttons get a strip of their own above the shell. |
| `desktop/desktop.css` | `:root[data-graphics="light"] *, :root[data-graphics="light"] *::before, :root[data-graphics="light"] *::after` | `animation`, `scroll-behavior`, `transition-property`, `transition-duration`, `-webkit-backdrop-filter`, `backdrop-filter` | The lighter look, for a desktop app whose page the processor draws: nothing moves and nothing blurs what's behind it. |
| `desktop/wrapper.html` | `:root.dk-transparent, :root.dk-transparent body` | `background` | The mini player's window is transparent: the player's own shape is the window, whatever wallpaper a design or theme paints. |

## 4. Tokens

Set any of these in `:root` (both modes) or `:root[data-theme="day"]` / `[data-theme="night"]`. The full list, with every default, is `web/tokens.css`.

| Category | Tokens |
|---|---|
| Page and panels (per mode) | `--sky` (page), `--sky-deep`, `--panel`, `--panel-2`, `--tray`, `--tray-edge` |
| Text and accents (per mode) | `--fg-sky`, `--fg-sky-2` (text on the page), `--fg-panel`, `--fg-panel-2` (on panels), `--link`, `--accent-ui`, `--heart`, `--focus`, `--edge` |
| Player (per mode) | `--seek-played`, `--seek-buffered`, `--seek-line`, `--play-ring`, `--sparkle`, `--speck`, `--q-lossless`, `--q-lossy` |
| Cards and screens | `--cream`, `--cream-2`, `--cream-3` (card paper), `--ink`, `--ink-2`, `--link-on-cream`, `--screen` (the device's LCD), `--line`, `--stack-1`, `--stack-2` (stack shadows), plus the palette (`--peri`, `--lav`, `--pink`, `--mocha`...) |
| Type | `--f-ui` (everything), `--f-px` (pixel face for machine facts), the scale `--t-display`, `--t-title-xl`, `--t-h2`, `--t-row`, `--t-body`, `--t-meta`, `--t-micro`, `--t-px` (each a `font` shorthand) |
| Space | `--sp-1` ... `--sp-12` (4 px steps in rem), `--gutter`, `--card-pad` |
| Radius | `--r-chip`, `--r-thumb`, `--r-art`, `--r-input`, `--r-panel`, `--r-card`, `--r-pill`, `--r-device`, `--r-bezel` |
| Stroke | `--stroke`, `--stroke-thin`, `--icon-stroke` |
| Shadow stacks | `--stack`, `--stack-sm`, `--stack-off-1`, `--stack-off-2` |
| Motion | `--dur-press`, `--dur-1` ... `--dur-4`, `--ease-out`, `--ease-in-out`, `--ease-spring` |
| Layers | `--z-bar`, `--z-panel`, `--z-sheet`, `--z-popover`, `--z-toast`, `--z-overlay` |
| Sizes | `--rail`, `--rail-icons`, `--ctx`, `--ctx-h` (the side panel's height where a theme docks it by its height, section 5), `--bar-h`, `--mini-h`, `--tab-h`, `--hit`, `--tag-h`, `--tag-ic`, `--tag-gap` (every tag's height, icon size, and gap between icon and words) |
| Window (section 12) | `--window-tint`, `--window-opacity` (from theme.json's `window`), `--window-backdrop` (what lies over the desktop where the wallpaper was) |

Components read semantic tokens (`--bg`, `--fg`, `--fg-2`, `--link`, `--edge`, `--focus`...) that the surfaces `.s-sky`, `.s-panel`, `.s-card` and `.s-screen` map from the ones above, so a surface's text always pairs with its own background.

The built-in designs draw from these tokens too, so a theme that `extends` one recolours its look with them. Petal's pixel frames are an example: the ring with cut corners round every window and control is `--line`, its hard stepped shadow is `--stack-2` (through `--stack`, `--stack-sm` and `--stack-1l`, which petal redefines as those pixel shadows), the paper inside is `--panel` and the dithered desk `--sky` with `--sky-deep`. Petal paints the ring in the background's border box, under the part's own paper, so recolour a framed part's paper with `background-color`: the `background` shorthand would erase its ring.

## 5. Stable hooks for full CSS

Go beyond tokens with any selector. These hooks are the theme API: they stay put across lull versions (section 9).

| Hook | Values |
|---|---|
| `:root[data-theme]` | `day`, `night` (always explicit; "match system" is resolved for you) |
| `:root[data-shell]` | `rail`, `topnav`, `dock` |
| `:root[data-motion]` | `reduce`, `full` (absent: follow the system). Keep motion off under `reduce`. |
| `:root[data-design]` | the built-in design that is loaded (your `extends`), else your id |
| `:root[data-window]` | `mica`, `acrylic`, `blur`, `transparent`: what the desktop app's window shows behind the page now (section 12); absent while the window is solid |
| `:root[data-graphics]` | `light`: the desktop app found the processor drawing the page, not a graphics card (Settings › App › Graphics). lull stops all movement and every `backdrop-filter` there, and decorative loops hold still; make sure your glass reads without its blur. Absent otherwise |
| `[data-region]` | `rail`, `header`, `topnav`, `content`, `ctx` (queue, lyrics, activity), `playerbar`, `mini` (the phone's player; on a phone held sideways, a window under 500 px tall, it stands in for the player bar everywhere but Now Playing), `tabbar`, `lead` (Home's homeLead slot) |
| `.app[data-screen]` | `home`, `search`, `now`, `library`, `liked`, `history`, `following`, `album`, `artist`, `tag`, `playlists`, `playlist`, `stats`, `settings`, `classic`... |
| `.app[data-route]`, `.app[data-view]`, `.app[data-ctx]` | nav key; `page` or `now`; `docked`, `open`, `closed` |
| `[data-act]`, `[data-nav]` | every control's action (`play`, `next`, `like`, `shuffle`...) and nav target (`home`, `library`...) |
| `#pbar` / `#mini` `[data-status]`, `[data-playing]` | player state (`idle`, `finding`, `queued`, `buffering`, `stalled`, `choose`, `streaming`, `library`, `failed`, `jellyfin-error`, `ended`); the desktop app's mini player windows have it too, on `[data-dk-player]` |
| `:root[data-player]`, `:root[data-playing]` | the same state and whether sound is playing, on the page's root, so anything anywhere can follow them (`:root[data-playing="true"] .my-moon`) without a `:has()` on the player |
| `#pbar`, `#mini`, `.now__dev`, `.now__devc` `[data-liked]`, `[data-saved]` | whether the playing song is liked and saved (`true` or `false`), on the player and Now Playing's hero; they change only with a like, a save or the song |
| `--song-pos` | where the song is, `0` to `1` in hundredths, written only when it moves a hundredth, and only on `#pbar`, `#mini`, `.sky-specks`, Now Playing's hero (`.now__dev`, `.now__devc`), Home's `.disc-lead`, the mini player windows' `[data-dk-player]` and any element of your slot markup marked `data-live` (section 14): draw with it there (`rotate: calc(var(--song-pos) * 1turn)`, `translate`, `scale`, `opacity`). It isn't inherited: a part inside them, or a pseudo-element, that draws with it says `--song-pos: inherit` (each level between too). So a step restyles only what draws with it, never a whole hero, player or page |
| `#pbar` / `#mini` / `[data-dk-player]` `[data-status-text]`, `--status-text` | the state's own words: what the state tab says, the hub's reason for a failure among them ("YouTube couldn't find this song"); while a song simply plays, where from ("Streaming from YouTube", "Playing from your library"). `content: var(--status-text)` shows them on anything inside (the property is a CSS string every element inside inherits); `attr(data-status-text)` only on the element itself. See **The player's words** below |
| `.mini__chip` | an empty part beside the mini player's song line, hidden unless your theme shows it and words it (`::before { content: ... }`, from `#mini[data-status]`); it gives way when the line needs the room, so never place it over the buttons |
| Classes | the BEM blocks: `.card` (`.card-head`, `.card-head__title`), `.btn` (`--primary`, `--quiet`, `--sm`), `.btn-icon`, `.btn-gloss`, `.cover` (`.c1`-`.c8` recipes, `.has-art`; `.cover--artist` with `.cover__mono`, the artist's initial shown while it has no photo, in the design's display face), `.trow`, `.tile`, `.chip`, `.pill-filter`, `.seg`, `.input`, `.seek`, `.wave`, `.device`, `.empty`, `.sheet`, `.dialog`, `.toast`, `.banner`, `.nav-item`, `.nav-pill`, `.tab`, `.region-title` (hidden window titles you can show). Every other class and attribute, stable or internal, is in section 11 |

**Wide screens (ADR-0007).** A bigger window reveals more (more columns, more tiles); nothing is stretched. Your theme can add its own touches per size:

| Hook | Values |
|---|---|
| `.app[data-size]` | the main area's width class: `compact` (under 640 px), `regular` (to 1099), `wide` (to 1599), `xwide` (to 2199), `ultra` (lull's widths: the `--layout-size-*` switches, below) |
| `.app[data-height]` | `short` (under 760 px, `--layout-height-short`) or `tall` |
| Containers | `main` (`.main`), `panel` (`.ctx`), `nowcol` (the Now Playing column), `section` (a Home module), `tlist` (a song list), `libpl` (the playlist table): `@container main (min-width: 1600px) { ... }` |
| `.app[data-panel="strip"]`, `.ctx__strip` | the side panel collapsed to an icon strip |
| `.nowcol`, `.nowcol__head`, `.nowcol__title`, `.nowcol__body`, `.app[data-nowcol="open\|strip"]` | the Now Playing column beside the main area |
| `.disc-modules > .disc-mod[data-module] > .card[data-region]` | Home's modules (`new`, `radio`, `picks`, `jump`, `tags`), in a fixed order. Each module's box names it, so `.disc-mod[data-module="new"]` places or groups one without a `:has()` |

**Layout switches (ADR-0101).** Layout is yours. Every choice about layout that lull's script makes is a switch: a
custom property on `.app` (or, for a row that fits itself, on the part or the row) that lull's CSS sets to what lull
does, at lull's widths, and that lull's script reads instead of deciding. Set any of them with any rule (a media query,
a screen, a size class, a state) and lull lays out by it; the gates still check that every control can be clicked and
every text read. They are registered: a value lull doesn't know is the switch's initial value (the first one in the
table). They aren't inherited, so set them on `.app` itself (`.app`, `.app[data-screen="home"]`, `@media (...) { .app
{ ... } }`), never on `:root`. lull reads them when the window resizes, the route or the design changes, a stylesheet
changes and a size class switches, never per frame; don't set one by a state it decides (`[data-ctx]`, `[data-nowcol]`),
which would flip it back and forth.

| Switch | Values | What lull does by it |
|---|---|---|
| `--layout-ctx` | `overlay` (lull: under 1360 px), `docked` (from 1360 px) | `docked`: the side panel (queue, lyrics, activity) docks in the shell's grid (`.app[data-ctx="docked"]`), and closing it folds it to its icon strip (`.app[data-panel="strip"]`); `overlay`: it opens over the page (`open`, `closed`). Where a docked panel sits is your grid's; lull's puts it on the right |
| `--layout-ctx-axis` | `inline`, `block` | what the docked panel's handle drags: its width (`--ctx`, from its left edge) or its height (`--ctx-h`, 200 to 600 px, from its top edge, where lull's CSS lays the handle, `aria-orientation="horizontal"`; up and down arrows). A panel docked by its height takes no width from the main area, so it never closes the Now column. Its strip is then `--ctx-h` tall: lay `.ctx__strip` out as a row |
| `--layout-ctx-dismiss` | `scrim` (lull: under 1100 px), `outside` (from 1100 px) | how the panel open over the page closes: its scrim (`.ctx-scrim`) takes the click, or a click anywhere outside it |
| `--layout-ctx-split` | `xwide`, or `compact`, `regular`, `wide`, `ultra`, `none` | the size class from which Queue and Lyrics can show as two panes (`.app[data-panel-split]`). Its button (`.ctx__split`) shows by `.app[data-size]` in lull's CSS: show it where you move this |
| `--layout-search-split` | `wide`, or `compact`, `regular`, `xwide`, `ultra`, `none` | the size class from which Search can show its lists side by side; its button is `.disc-split`, likewise |
| `--layout-nowcol` | `room`, `kept`, `open` | the Now column. `room`: offered (`.app[data-nowcol-ok]`, its button) while the main area stays `wide` beside it, and opening it or the docked panel closes the other when both don't fit. `kept`: offered at every width, and neither closes the other. `open`: open on every screen but Now Playing (or at the listener's strip), with no button to close it |
| `--layout-phone` | `off`, `on` (lull: under 640 px) | lull's phone behaviour: the queue opens on Now Playing's Queue tab and asking for the lyrics there goes to its Lyrics tab, the equaliser and Play on another device open as sheets, Esc on Now Playing goes back, Classic starts with its equaliser closed. Your CSS draws the phone's layout itself (lull's, under 640 px: the mini player and the tab bar); set this where it does |
| `--layout-size-regular`, `--layout-size-wide`, `--layout-size-xwide`, `--layout-size-ultra` | any length: `640px`, `1100px`, `1600px`, `2200px` | the main area's width from which each size class starts (`.app[data-size]`); `wide` is also the room the Now column needs |
| `--layout-height-short` | any length: `760px` | the window's height under which `.app[data-height]` is `short` |
| `--layout-panel-min`, `--layout-panel-max` | any length: `280px`, `560px` | the docked panel's width the listener drags within (their own width is kept per device, held to these) |
| `--layout-panel-h-min`, `--layout-panel-h-max` | any length: `200px`, `600px` | its height, when docked by its height |
| `--layout-now-min`, `--layout-now-max`, `--layout-now-width` | any length: `300px`, `560px`, `360px` | the Now column's width the listener drags within, and its width until they do |
| `--layout-strip` | any length: `60px` | a folded panel's or Now column's strip |
| `--layout-nowcol-gap` | any length: `16px` | the gap lull counts beside the Now column when it measures the room the column takes: set it with your grid's gap there |
| `--layout-popover-margin`, `--layout-tip-margin` | any length: `12px`, `8px` | how far a popover (a menu) and a tooltip keep from the window's edge |
| `--layout-popover-side` | `below`, `above` | the side of its key a popover opens on when it has room there (else the other) |
| `--layout-window-controls` | `room`, `own` | the desktop app's minimise, maximise and close buttons on Windows, over the window's top right corner. `room`: lull keeps the header clear of them (its right padding, or a strip header's margin, `!important`) and gives them a strip of their own from 640 to 819 px. `own`: lull reserves nothing (`.app[data-window-controls="own"]`), so your header, column or picture can run to the edge under them, and you keep your keys out of their box: `--window-controls-w` × `--window-controls-h` from the top right corner (138 × 32 px on Windows, `0px` everywhere else). The buttons always stay on top and clickable |
| `--fit-order` | `auto` (the part's `data-fit`), or a whole number | on a part that gives way: its order in its row, 1 first. Parts of one order give way together, and the row's `data-fit-gave` lists these orders |
| `--fit-sides` | `mirror`, `none` | on a row in three columns (`#pbar`): `mirror` sizes both sides alike (`--fit-side`, as wide as the tools need), so the transport stays centred; `none` leaves the sides to your grid, and lull sets no `--fit-side` |

The desktop app's mini player window takes its height from what your mini player draws, and its width from
`--layout-mini-width` on `[data-dk-player]` (a length; the app's own when you set none: 340 px for the device, 540 for
the strip), kept within the screen.

The queue along the bottom, docked at every width from 1000 px and dragged by its height, a Now column that never
folds, and Like and Save the last to give way, after the song's chips (`display: contents` on a wrapper hands its parts
to the row around it, which the fit follows, so the chips and Like and Save give way in one order):

```css
@media (min-width: 1000px) {
  .app { --layout-ctx: docked; --layout-ctx-axis: block; }
  .app[data-ctx="docked"] {
    grid-template-columns: var(--rail) minmax(0, 1fr);
    grid-template-rows: auto auto minmax(0, 1fr) var(--ctx-h-drag, var(--ctx-h)) auto;
    grid-template-areas: "rail head" "rail banner" "rail main" "rail ctx" "bar bar";
  }
  .app[data-ctx="docked"] .ctx { width: auto; }
}
.app { --layout-nowcol: open; }
.pbar__badges { display: contents; }
.pbar__acts { --fit-order: 9; }
```

Where lull's script decided each one before (the audit for ADR-0101; `mise run check` now fails a width, a height or a
breakpoint compared in `web/` script, `tests/web/layout-switches.test.mjs`, which lists the few measurements that
decide no layout: keeping a popover or tooltip inside the window, a list scrolled while a row is dragged, the desktop
mini player's window sized to what it draws):

| Where | What it decided | lull's rule | Switch |
|---|---|---|---|
| `app.js` `App.openPanel`, `layoutCtx` (on resize) | whether the side panel docks or opens over the page | window from 1360 px | `--layout-ctx` |
| `app.js`, a click outside the open panel | whether it closes the panel | window from 1100 px | `--layout-ctx-dismiss` |
| `screens/now.js` `now-go-search`, `now-job-review` | close the panel before leaving | window under 1360 px | `--layout-ctx` |
| `layout.js` resize handles | the panel's handle drags its width | always | `--layout-ctx-axis` |
| `layout.js` `apply` | Queue and Lyrics as two panes | main area `xwide` | `--layout-ctx-split` |
| `screens/discover.js` `splitView` | Search side by side | main area `wide` | `--layout-search-split` |
| `layout.js` `apply`, `makeRoom` | the Now column offered, shown, closed for the panel | main area stays `wide` beside it | `--layout-nowcol` |
| `layout.js` size classes | `.app[data-size]`, `.app[data-height]` | 640, 1100, 1600, 2200 px wide; under 760 px tall | `--layout-size-*`, `--layout-height-short` |
| `app.js` `showQueue`, Esc; `screens/discover.js` `isPhone`; `screens/now.js` (`?tab=queue`, a card scrolled into view, `now-open-queue`, the lyrics asked for) | the phone's way: Now Playing's tabs, Esc goes back | window under 640 px | `--layout-phone` |
| `connect.js` Play on another device; `screens/eq.js` the equaliser | a sheet or a popover | window under 640 px | `--layout-phone` |
| `screens/classic.js` `createWebamp` | Classic starts with its equaliser closed | window under 640 px | `--layout-phone` |
| `layout.js` `fit` | the order parts give way in | `data-fit` | `--fit-order` |
| `layout.js` `settle` | the player bar's sides mirrored | `[data-fit-sides]` | `--fit-sides` |

**Never patch a width band.** The header, the player bar, its tools and the song's chips fit the room they actually
have (a docked panel, the Now column, the text size and a long title count as much as the window): each part that can
give way has an order (`data-fit`, or your `--fit-order`), and when a row runs out of room its parts give way in that
order and come back with room. So a theme never writes `@media (min-width: 1100px) and (max-width: 1359px)` to hide a
tool or shrink a pill: style `[data-fit-out]` for how a part looks once it gave way, give the parts your own order with
`--fit-order`, and, while the bar keeps lull's three columns, leave them to lull (both sides are `var(--fit-side)`, as
wide as the tools need, so the transport stays centred; a design may set `--fit-centre-min` and `--fit-side-min` on
`.pbar`) or end them with `--fit-sides: none` and lay the bar out your way; let a part's track be `auto`, so a part that
gives way takes its room with it. To move a choice lull makes at a width (where the panel docks, when the phone's
behaviour starts, the size classes), set its switch rather than patching around it; other width rules are for your own
layout and for content that rearranges by its container's room (`@container`). `mise run check:ui` sweeps every width
from 360 to 2560 px for the built-in designs (docs/context/quality.md#ui-gates).

**The shape is yours; the gates check what works (ADR-0083).** Where a part sits, whether it rises out of its bar,
whether a bar is full width or a rectangle at all (a capsule narrower than the window, standing on a floor), whether one
control sits inside another's area (a play key at the centre of the song's disc, a seek drawn as the capsule's tube), and
whether a region keeps lull's usual shape: none of it fails a gate. What does, at every width from 360 to 2560 px:
- a control nothing can click: off the screen, or under something that takes every point of it (a glow or a sheen
  drawn over it that catches clicks; give a layer that only decorates `pointer-events: none`);
- text under something that paints over it (a background, a picture; an empty box doesn't count), text drawn on other
  text (letters on letters, not line boxes), text off the screen, and text cut without an ellipsis.
  A page is judged once it has arrived: an entrance that slides it in from off the screen is fine, and so is content
  that scrolls under a part laid over its scroller (the page under the chrome, lyrics under a foot), since it scrolls
  clear;
- a feature nothing reaches. lull's features are listed in `web/designs/features.json`, each with the controls that reach
  it and where it is expected: play and pause, previous, next, seek, the volume, Like, Save, shuffle, repeat, the queue,
  the lyrics, Activity, Now Playing, the player's "…", Search, Home, the Library, Playlists, Stats, Settings, playing on
  another device, the equaliser, Classic mode, the mini player (the desktop app) and the design picker. On every screen
  where lull offers one, a control a click reaches (shown, at least 24 × 24 px, not covered at every point) must give
  it, or it must be in a menu or sheet one such click opens: the player's "…", the phone's More sheet, Following and
  playlists, and on a phone Now Playing, the player's sheet there. So hide, move, group, fold or redraw lull's
  controls as you like; just leave every feature one click away;
- a control under 24 × 24 px or anything else the accessibility gate (axe, WCAG 2.2 AA) finds, and the order the
  keyboard and a screen reader take, which is lull's markup order whatever the CSS moves.

A few checks guard lull's own look and can't tell a choice from a bug. Where your design differs from them on purpose,
free it with `--gate-free` in your CSS, on the part or a box round it, naming the check and saying why (a reason of a few
words; without one the check still runs, and says so). Several go in one string, separated by `;`:

```css
#pbar { --gate-free: "song-side-room: the capsule is narrower than the window, so the chips go to the menu"; }
.pbar__badges { --gate-free: "even-tags: the chips lean like cards on a desk"; }
```

| Check | What it expects of lull's look |
|---|---|
| `even-tags` | tags side by side are one height on one centre line, with one icon size (**Tags**, below) |
| `song-side-room` | at 1440 px the player bar's song side keeps its chips, Like and Save, rather than giving them to the "…" menu |
| `seg-indicator` | a segmented control's indicator lies on (or just under) the picked choice (**Measured parts**, below) |


Prefer `@container main` to `@media`: it follows the main area, so it stays right when a panel opens. Layout states that must hold whatever a theme draws (the strips, the room the column takes) are `!important` in lull's own CSS: style them, don't fight them. The widths the listener drags are custom properties (`--ctx`, `--now-w`, and `--ctx-h` for a panel docked by its height); read them, don't set them (`--ctx` and `--ctx-h` are tokens too: a design's own starting size). While a drag runs the column's width is `--ctx-drag` / `--now-drag` instead (`--ctx-h-drag` for the height), so a theme's own shell grid that places the panel writes its track as `var(--ctx-drag, var(--ctx))`.

`design/DESIGN.md` section 3.4 lists more state hooks (tray pills, track row file facts). Pseudo-elements are free for ornaments; keep them decorative.

**Measured parts.** lull places a `.seg`'s indicator (`.seg__ind`) on the selected button by measuring it (`--seg-x`, `--seg-w`), so gaps, padding and labels of different lengths are fine: restyle or hide the indicator, but don't position it. A design that shows the pick another way (a dot beside it) names `seg-indicator` in `--gate-free` (above). A `.wave`'s bars share its whole width (set the `gap`, not the bars' width), and its head travels the strip less its own width, so it never passes the last bar: a bigger head sets `--wave-head` on the `.wave` and fills `.wave__rail`.

**Staying fast.** Chrome re-checks a `:has()` on a page-wide element (`:root`, `.app`, `.main`) whenever anything is inserted anywhere: a menu, a toast, a tooltip, a row. Each time it restyles every element the rest of the selector could match, so `:root:has(X) .now-card :is([data-act="retry"])` or `... span:not(:last-child)` restyles the whole page on every such change. End those selectors in a class, prefer a state lull already sets (`.app[data-screen="now"]` rather than `.app:has(.now)`; `.pipeline-wrap--idle` for a save pipeline that hasn't started; `.wave-times__buf` for the "buffered to" part of the times), and never test classes by substring (`[class*="is-"]`) inside `:has()`. Animate `transform` and `opacity` only, so loops run off the main thread, and draw them cheaply (below). `mise run check:ui` measures all of this for the built-in designs (docs/context/quality.md#ui-gates).

**Drawing cheaply.** A loop that never ends (a twinkle, a blinking caret, a ticker, the playing bars) keeps the GPU drawing while the listener is away, as often as the screen refreshes (165 times a second on a fast monitor) unless it says otherwise. Keep your loops, and make each one cheap:
- **Step it where you declare it.** Give the loop a stepped timing function, so it changes only as often as its motion needs, from its first frame, in every engine and whatever the screen's refresh rate: `steps(n)` for linear motion (a spinner, a ticker), a stepped easing for an eased one. lull's are custom properties named for the curve and the steps, `--loop-ease-in-out-55` (CSS's `ease-in-out` sampled 55 times an iteration: 10 changes a second over 5.5 s) or `--loop-in-out-52` (lull's `--ease-in-out`): `animation: twinkle 5.5s var(--loop-ease-in-out-55) infinite alternate`. `node scripts/loop-easings.mjs ease-in-out 70` prints one for your own `:root[data-design]` block (n = seconds × changes a second), and `--check web` keeps every one exact. Pick the slowest rate that looks the same: 10 a second for a slow twinkle or fade (lull's sparkles, Skydeck's sky), 20 for a quicker twinkle or a ticker (about a pixel a step), 30 for what moves with the music (the playing bars). A blinking caret's `steps(1)` is already cheap. Or name the rate and let lull step it: `--loop-fps: 10` on what the loop animates, the way themes have done it so far. lull steps such a loop as it starts (keeping its easing), so it can draw a few frames unpaced first where an engine is slow to say a loop started; a stepped timing in your CSS is cheap from its first frame.
- **Damage only what moves.** Animate `transform` and `opacity` only, on the smallest box that moves. A glow is its own pseudo-element that fades and grows, never an animated `box-shadow`, `filter`, `background-position` or `clip-path`: each of those repaints on the main thread at every step. Skydeck's down LED is the pattern: the pill draws the glow on `::before`, anchored to the LED, and breathes it by transform and opacity. The same goes for a scroll-driven effect (an `animation-timeline` of `scroll()` or `view()`, a fade at a list's foot): animating anything else, a custom property included, puts it on the main thread, which runs it at every frame, scrolling or not. Fade it by opacity on a layer of its own (lull's `.view::after`).
- **Keep loops off glass.** A `backdrop-filter` blurs again whatever changes under it or inside it, every frame: a twinkle under Glasshouse's frosted queue kept the GPU fully busy. No loop sits under, or inside, an element with a live `backdrop-filter`.
- **Bake what's static.** Glass that stays put over your own wallpaper (the shell's panels) needs no live blur. Draw the wallpaper's smooth layers again under the glass's tint, with `background-attachment: fixed` so they line up with the wallpaper behind, and leave out the fine detail (stars, thin rings) that a wide blur wipes out anyway (Glasshouse's `--gh-wall`). A wallpaper of many full-window gradients repaints every glass window that redraws it on each resize: bake its smooth layers into a picture in `assets/`, stretched to the window, and keep only px-sized layers as gradients (Glasshouse's `assets/wall-*.webp`, and its star tile). Gradients, shadows and noise that don't move are drawn once and kept; they cost again only when something animates them or something under a filter changes. Glass that moves (a window on the scrolling page) keeps its live blur, since a fixed background under it repaints it on every scroll frame, unless a loop lives in it (Glasshouse's AIMP window and its ticker): then bake it, as one repaint while scrolling costs less than a blur redone at every step. Keep a live `backdrop-filter` wherever the page's own content passes under it: a sheet, a menu, a toast, a panel that slides over the page.
- **Hold still when asked.** Add `animation-play-state: var(--loop-play, running)` after a decorative loop's `animation`: it pauses while `:root.is-still` (lull's one rule for every reason to: so far the listener's "Pause animations when lull isn't the active window", Settings › Appearance, off by default), from the loop's first frame. A more specific rule of yours (a ticker paused on hover) still wins, and entrances, which don't use it, still finish. Leave it off a loop that shows something happening (a spinner, a loading sheen, a transfer). A loop that names its rate with `--loop-fps` holds by itself; give it `--loop-holds: no` if it shows progress. A loop that does neither keeps going. Style `:root.is-still` yourself only to still more.
- **Keep big decoration in the page's own layer.** Put your wallpaper and its specks on `.sky-specks`, which lull places absolutely under the app (the page never scrolls, so it covers the window) and clips at the window's edge. A `position: fixed` layer as large as the window becomes a compositor layer of its own, and the app painted over it needs another as large: each is a window-sized texture in GPU memory, drawn again every frame. Animate only what has to move; an animated layer as large as the window (Skydeck's twinkling sky) costs a window too, so keep it to one.
- **A mask goes on a box, never on inline text.** WebKit (lull's Linux app, Safari) paints a mask on an inline box inside out: the whole box in its background, with the mask's shape cut from it. Give a masked element or pseudo-element `display: inline-block` or `block`; where it must stay inline (a heart that rides the end of a title's last word, glued by a word joiner), draw the shape from background rectangles instead, each pixel a whole screen pixel (`round(…, 1px)`), as Petal's title heart does. `gates/masks.spec.mjs` checks the built-in designs.
- **Theme Studio checks it with you.** It names each loop that never ends and costs more than it needs to, says what that costs and the cheaper way, and so does Settings › Appearance when you import a theme. It's a hint: the theme works either way.
  - **No rate:** a loop neither stepped and held where it is declared nor naming `--loop-fps`, with the rate this guide suggests (20 when it can't tell what the loop shows, and it says so), and an Add it that writes `--loop-fps` after the loop's `animation`. A rate you add or change takes hold at once, in Studio's preview and on open devices when the theme reloads.
  - **Redrawn at every step:** a loop whose keyframes animate anything besides `transform` and `opacity` (`translate`, `scale` and `rotate` count as transform), with the cheaper way for what it changes: a glow on a `::before` that fades and grows, a pattern moved by `transform`, motion by `translate()`, growth by `scale()`, a colour faded in on a layer. There's no Add it: no rewrite of these is sure to look the same.
  - **Under glass:** a loop inside an element with a live `backdrop-filter` (its own rule blurs, or the selector puts it inside one that does, in your theme or the design you extend, as the cascade leaves it), and, in Studio, a loop its day and night previews show passing under glass on the screen they show (switch the screen to check others). From the CSS alone nobody can tell what passes under what, so an imported theme with glass is told that, and pointed to Studio. The advice is the two above: move the loop out, or bake the glass.
  - **What it reads:** the rule the loop is in, its `@keyframes`, a rate or a hold set on the same selector elsewhere in your theme, in lull's own CSS or in the design you extend (a theme that replaces one of lull's animations replaces its steps too), and `:is()` and `:where()` lists written out. A loop that an `@supports` block switches off (`animation: none`) on the same element is a fallback for browsers without what its better version needs, and is left alone (Skydeck's down LED).
- **The gates** check all of this. gates/perf.spec.mjs checks at rest, with a song paused in day and night, and while playing on Now Playing: every loop on screen changes at most 30 times a second, moves only by transform and opacity, and stays clear of live glass; every scroll-driven effect on screen animates only transform and opacity; a page at rest draws at most 22 frames a second; and the compositor keeps no more than the design's budget of layers, counted in windows of area (each layer up to one window). gates/loops.spec.mjs checks, in Chromium and in WebKit (Linux's app, Safari), that every loop is stepped from its first frame and holds still when asked. A rate is changes per second of motion, so a loop draws the same frames on a 60 Hz screen as on a 165 Hz one.

**Cover variety:** with `"coverVariety": true` every cover gets stable per-item hooks, so two albums that share a generated cover still differ: `data-look="0"`-`"5"` (pick one of 2, 3 or 6 looks with `[data-look="1"], [data-look="3"], [data-look="5"]`, ...) and `--cover-flip` (1 or -1), `--cover-hue` (-28deg to 28deg), `--cover-zoom` (1 to 1.45), `--cover-dx`, `--cover-dy`.

**Parts with states of their own.** These hold rules a theme must keep, so style them, don't rebuild them:

- **The song row's number cell** (`.trow__num`) shows one thing at a time: the number (`.trow__n`), Play (`.trow__play`) under the pointer or keyboard focus, or the playing bars (`.eq`, drawn in `.trow.is-playing` only). Under 520 px the cell is gone and the cover is the playing marker (`.trow.is-playing .trow__art::after`).
- **The player's words.** A state's reason changes from song to song: a song fails because no peer answered, because YouTube couldn't find it, or because the library's file is gone, and it streams or buffers from a peer, YouTube or the library. So don't write a reason of your own for a state (`"no peer answered"` is wrong for every other failure): show lull's, `--status-text`, or words that name no cause ("stopped", "couldn't play this song"). For the failed state, keeping your own warning sign:
  ```css
  .pbar[data-status="failed"] .pbar__seek::before { content: "\26A0  " var(--status-text); }
  ```
  `--status-text` is set on `#pbar`, `#mini` and the mini player windows' `[data-dk-player]`, so it reaches what is drawn inside them; outside them (Now Playing's own parts), say what happened without a reason. Theme Studio and importing a theme name a rule that gives a cause of its own for such a state (failed, jellyfin-error, stalled, choose, finding, queued, buffering, streaming), and where it draws inside the player, **Use lull's own words** puts `var(--status-text)` in its place. It's a hint: the theme works either way.
- **The playing bars** (`.eq`, three `<i>`) move only inside `.is-playing-now` (sound is coming out), and stand still while paused. Animate them with `transform` only.
- **The place playing** (a tile, a rail playlist, a search result, a menu item: `[data-place].is-playing-place`) gets `.place-mark` (the bars, plus a hidden "(playing)") before its title. "Playing from" is `.from` (icon, `.from__label`, the place's name in `b`): `.pbar__from` in the player bar, `.mini__from` in the phone mini player, `.now-head__from` on Now Playing. Clicking it opens the place, where the playing row is lit for 1.8 s with `.is-revealed`.
- **Covers.** `.cover` holds a recipe (`.c1`-`.c8`, drawn on `::before`/`::after`) until its picture loads; then `.has-art` hides the recipe. A cover still waiting for the hub keeps its recipe, so a recipe must look finished on its own. `.mosaic` is a playlist's 2×2 of covers, and `.mosaic--one` holds one picture: the playlist's own, or the one album its songs share.
- **The cover viewer.** `.cover-zoom` wraps the player bar's and the Now column's cover, and its `.cover-zoom__btn` shows on hover and keyboard focus. The button is never shown on touch screens. The viewer is a `.dialog.cover-view` holding `.cover-view__art` and `.cover-view__info` (`__title`, `__by`). Its picture is as large as the window allows, up to 1024 px: give it a frame, not a size.
- **Sortable column headers** on the playlist page (`.lib-thead > .lib-th`) carry `aria-sort="ascending|descending"` on the sorted column. The arrow is `.lib-th__btn .ic`, and `.lib-th--added` is the date column. While the list is sorted, `.lib-sortnote` says why dragging is off.
- **Tags** (`.badge`, `.q-chip`, `.badge-new`, `.device__fact`, `.pbar__private`, `.mini__private`) are one box: `--tag-h` tall, border-box, contents centred, an icon `--tag-ic` square and `--tag-gap` from its words, so tags side by side are one height on one centre line with one icon size. Size them with these tokens, on `:root` or on the row that holds them (`.tile__meta`, `.pbar__badges`), never with a tag's own `height`, `gap` or icon size. A wrapper that holds only a tag (`.src-badge`) is only that tag's box. Tags set apart on purpose name `even-tags` in `--gate-free` (above).
- **The player bar** is laid out as three columns (`.pbar__now`, `.pbar__centre`, `.pbar__tools`) with equal sides, so the transport stays centred; that is lull's layout, not a rule: a design may shape and place the bar and its parts as it likes (a capsule, the play key in the disc), as long as each control can still be clicked. Toggles show pressed or not (`aria-pressed`). Only width breakpoints move tools into the "…" menu (`.pbar__tools .is-wide` are the ones that go). The status line is `.pbar__state`, a tab on the bar's top edge that is `[hidden]` while all is well, with `.is-error` for a failure. `.pbar__xfade` floats above the tools during a crossfade, and `.pbar__private` is the "Private" tag. The two times are as wide as the song's duration reads (`--time-ch`), so the seek bar keeps its length.
- **The phone's player bar** (`#mini`, a row that fits itself like the player bar's) holds the cover (`#mini-thumb`), the song (`.mini__text`), and the keys: play on another device, previous, play, next. Previous, play and next never give way; the key for another device does (`data-fit="1"`, `[data-fit-out]`) when the song's words would be squeezed under `--fit-keep` (100 px), and Now Playing's "…" has it. Each key is at least `--hit`. `--mini-inset` is the room at both ends of the bar, between its edge and the cover on the left and the last key's icon on the right (the key's box, `--hit` wide, takes the rest of that side's padding): keep the two equal. The progress line (`.seek--line`) is the bar's seek, a slider you drag with a finger: its touch area is its own box, a transparent border `--hit` tall round the line (`--seek-up` above it, `--seek-down` below, which a negative margin takes back, so `top`, `bottom` and `height` place the line itself), reaching past the bar's edge and sitting under the bar's own parts, which keep their taps, so the bar does not clip (`overflow: hidden` would cut the touch area off); if you move the line, set `--seek-up` and `--seek-down` so it reaches where nothing else is, and leave its border alone. A swipe along the bar skips songs; a drag that starts on the line seeks instead.
- **Sliders** (`.seek`, `.range`, `.vol`): `.seek.is-scrubbing` while the pointer drags it. A slider clicked rather than tabbed to has `[data-pointer]` and no focus ring; the ring comes back with the first key. Show focus with `:focus-visible`, and never on `[data-pointer]`.
- **Segmented controls** (`.seg`): place `.seg__ind` by the measured `--seg-x` and `--seg-w` (above).
- **The side panel's tabs** (`.ctx__head .seg__btn[data-ctx-tab="queue|lyrics|activity"]`) and the phone Now Playing tabs (`.now-seg`) lose Lyrics for a song with no words (none found, or instrumental), and get it back when a song has some. Never give a tab a fixed position by count.
- **Classic's section in Settings** (`/settings/appearance`): `.cls-now` (the skin in use), `.cls-browser` (tabs and search over `.cls-grid`), `.cls-card` (`.is-applied` for the skin in use, `.cls-card__status.is-bad` for one that failed) with its `.cls-shot`, the larger view `.cls-pop` > `.cls-look`, `.cls-nsfw`, and Your skins' `.cls-folder` (where the hub's folder is, with Look again) and `.cls-skipped` (the files it skipped). Keep `.cls-shot`'s 275:348 ratio and its sharp pixels. The stage itself is Webamp alone on `.cls-desk` / `.cls-scrim`; the app under it is hidden while `:root[data-classic="on"]`.
- **The desktop app's own pages** load your theme too: the mini player windows (`.dk-mini`, and `.dk-strip` in `.dk-strip-wrap`, with `slots/miniPlayer`), and the connect, unreachable and close-to-tray pages (`.dk-page`, `.dk-card`, `.dk-form`, `.dk-hubs`, `.dk-modal`). They come from the app, not your hub, so the app's window hands them your theme once it has loaded it: `theme.css` with every `url("assets/…")` inlined, and your slots. Your theme shows in the mini player, and on the unreachable page when the hub goes down while lull runs. An edit on the hub reaches an open mini player. Until a page has your theme, it shows the design you `extends` (lull when you extend none): after the app starts with the hub down, or for the moment before the window has loaded it. Reach your files only through `url()` in `theme.css`: that is what gets inlined. The title bar is `.dk-titlebar` with `.dk-caption` buttons, under `:root[data-titlebar="win|gnome"]` and `:root[data-window-inactive]`. On Windows the caption buttons sit over the page's top right corner (138 × 32 px), and the app keeps your header clear of them (`desktop.css`, `!important`, so a header margin or padding you set can't put your keys under them). To fill that corner yourself (a column or picture that runs to the window's edge under the buttons), set `--layout-window-controls: own` on `.app` (section 5, Layout switches): lull then reserves nothing, and you keep your keys out of `--window-controls-w` × `--window-controls-h` at the top right. The mini windows are transparent (`:root.dk-transparent`), so the player's own shape is the window: never paint the page `body` there. The Classic window (`desktop/winamp.html`) is Webamp on nothing and takes no theme.

## 6. Slots: your own markup

A template in `slots/` is markup of yours. Six slots replace a piece of lull's:

| Slot | Where | Variants (`slots/<slot>.<variant>.html`) |
|---|---|---|
| `brand` | the wordmark in the rail, header and desktop pages | |
| `emptyArt` | the picture of every empty or error state | |
| `homeLead` | Home, between the greeting and the shelves | `playing`, `idle` |
| `nowHero` | the Now Playing hero | `full`, `compact` (phone tabs, short screens) |
| `miniPlayer` | the desktop app's mini player windows | `device`, `strip` |
| `coverArt` | every cover | no template: use `coverVariety` |

lull looks for `slots/<slot>.<variant>.html`, then `slots/<slot>.html`, then your `extends` design's, then its own. An empty file renders nothing (an empty `homeLead.idle.html` hides the lead while nothing plays).

Two more put markup of yours at the edges of any region or screen of lull's page, where lull has nothing to replace (ADR-0101):

| Slot | Where | Names (`slots/<slot>.<name>.html`) |
|---|---|---|
| `before` | the first thing inside a region or a screen, or just before the whole shell | `app`; regions: `agg`, `aggs`, `content`, `ctx`, `details`, `folder`, `header`, `jump`, `lead`, `mini`, `new`, `nowcol`, `picks`, `playerbar`, `radio`, `rail`, `recents`, `rels`, `results`, `save`, `srcmeta`, `srcrows`, `sub`, `tabbar`, `tags`, `topnav`; screens: `home`, `search`, `now`, `library`, `liked`, `downloads`, `history`, `following`, `album`, `artist`, `tag`, `playlists`, `playlist`, `outside`, `stats`, `settings` |
| `after` | the last thing inside it, or just after the whole shell | the same |

- **A region slot** (`slots/after.playerbar.html`) is drawn in a box of its own, `<div class="region-slot" data-slot="after.playerbar">`, as the region's first child (`before`) or its last (`after`). A region is any element lull draws with `[data-region]`: the shell's (section 5), and the screens' own, Home's shelves (`picks`, `new`, `radio`, `jump`, `tags`) and lead among them, Search's results, a source's details. lull draws the box with the shell or with the screen that holds the region, and never again for a song or a state: what changes reaches it through the live model (section 14) and lull's parts (below), in place. Nothing lull draws in a region replaces it, also when lull draws that region again on its own (a shelf the feed refreshed): lull's redraw keeps every `.region-slot` where it is.
- **The whole shell** (`slots/before.app.html`, `after.app.html`): a box just before or after `.app`, outside its grid, for a strip above or below everything. lull sets no layout for it, and `.app` keeps its full height: make room in your CSS (`body { display: flex; flex-direction: column; height: 100dvh; } .app { height: auto; flex: 1; min-height: 0; }`).
- **A screen slot** (`slots/before.home.html`) is drawn in the same box inside the screen's own top container: Home's `.disc-home`, Now Playing's grid `.now__grid`, Search's `.disc-search`, the library's pages' `.view__inner` (Library, Liked, History, Following, Downloads, album, artist, tag, playlists, playlist, outside), Stats' `.sys-stats`, Settings' `.sys-settings`. It is drawn with the screen, each time lull draws it, so your box is an item of the screen's grid beside lull's parts, placed by the same grid.
- **You place the box.** A region lays out its children its own way (the player bar is a grid of three columns, the mini player a row), so give the box a place there (`#pbar > [data-slot="after.playerbar"] { grid-column: 1 / -1; }`), take it out of the flow, or say `display: contents` to hand what's inside to the region's own layout. A box you don't place lands where that layout puts a new child. What CSS moves is still read in the markup's order: a `before` box is read, and tabbed through, before the region.
- **A theme with none adds nothing**, not even the box. A `before` or `after` always names its region or screen: the hub refuses `slots/before.html` and a name lull doesn't have, and says which there are.

**lull itself, not a menu** (ADR-0101, amended again). Four parts reach everything lull holds and draws, so a mockup
needs nothing added to lull:

**1. Any list.** `<lull-part name="list">` draws any list of lull's records:
- `of="queue"`: the songs after the one playing, each with its `group` (`yours` for those the listener queued, `place`
  for the place it plays from, `radio`);
- `of="home.jump"`, `home.new`, `home.picks`, `home.radio` (any key of Home's feed), `of="history"`, `of="liked"`;
- `of="activity"`, `of="playlists"`, or any other list lull keeps in the page, drawn again whenever lull changes it (an
  Activity row is its song, with the job's own `state`, `pct` and `peer`; `item.entry.kind` is the job's kind);
- `read="<path>" key="<field>"`: the items at `key` of any read of lull's API under `/api/` (docs/context/api.md), as
  `read="/api/library/albums" key="items"` or `read="/api/search?q=city+pop" key="albums"` (`key` may go down through
  lists: `days.items`).

An item is the record its id names (lull's ids say their kind: `t-` a song, `al-` an album, `ar-` an artist, `pl-` a
playlist), with what the list adds beside it (a play's `at`, a queued song's `group`), and the fields lull works out: a
song's `source` (where it would play from: `library`, `device`, `soulseek`) and `liked`; an album's, an artist's or a
playlist's `place`. A queued song has `qi`, its place in the queue. `item.entry` is the list's own entry as it came (a
job's own `kind` where the song's record has its own). `where="group=yours"` keeps
the items whose field says so (`group=place|radio` for either; several tests with commas; a field may go down:
`seed.kind=artist`), `sort="title"` (or `-duration`, descending) orders them, `count` takes the first so many (50 if
absent, at most 200), `label` names the list for a screen reader, `empty` says what shows when nothing is left. A read
is fetched where the list is placed, once, and the list is drawn again each time lull reads it again.

Without a row of yours the items are lull's own rows. With one, `<lull-row>` inside the part, each item is drawn in
your markup, in an `<li class="lull-list__row">` of an `<ol class="lull-list">`: `{{item.<field>}}` fills a field of it
with pipes (section 14), and every part inside the row is that item's (below):
```html
<lull-part name="list" of="queue" where="group=yours" label="Your next songs">
  <lull-row><lull-part name="cover" size="40"></lull-part>
    <b>{{item.title}}</b> <i>{{item.source | map:library=● LIB,soulseek=◆ SLSK,device=▼ DEV}}</i> <i>{{item.duration | time}}</i>
    <lull-part name="like-button"></lull-part><lull-part name="play-button"></lull-part></lull-row>
</lull-part>
<h3>Next from <b data-bind="radio.name"></b></h3>
<lull-part name="list" of="queue" where="group=radio" count="4"></lull-part>
```

A row of an album, an artist or a playlist is marked while the song plays from it, as lull's own rows are, the moment
it is drawn: `.is-playing-place`, and `.is-playing-now` while it plays, with lull's mark (put `data-place-at` on the
element of your row it belongs in).

**2. Any part for any item.** `for="<id>"` makes a part that item's, anywhere; inside a list's row it is the row's
item by itself, with what the list adds (a queued song plays where it is in the queue, and its menu is the queue's).
`cover` (`size`; a playlist's own picture or its songs' mosaic), `play-button` (plays the song, the album or the
playlist; starts an artist's radio),
`like-button`, `save-button`, `source-badge`, `quality-chip` and `menu` (the song's "…") each draw lull's own control
for it, wired and kept current as lull's own rows are. Without `for`, a part is the playing song's, as before.

**3. Any piece of any screen.** `<lull-part name="screen" path="/home" pick='[data-region="jump"]'>` draws the screen at
`path` as lull draws it and keeps the piece `pick` names (any selector), in any slot: Home's "jump back in" in the side
panel, Now Playing's queue on Home (`path="/now" pick='[data-now="queue"]'`), an album's tracklist beside the player
(`path="/album/al-…" pick=".tlist"`). It is drawn again when what that screen read comes back and at the player's
changes, never on a tick. Its buttons work by lull's actions; what a screen does only while it is the one shown (a
drag to reorder, a scroll it keeps) stays there. `home.picks`, `home.new`, `home.radio`, `home.jump`, `home.tags`,
`now.queue` and `now.source` are short names for the pieces used most.

**4. State, for your own markup.** One action keeps a state, with no script: `data-act="state"` on a key of yours, and
`data-set` for what it does:
- `data-set="tab=lyrics"` sets a state to a value;
- `data-set="drawer"` turns one on or off;
- `data-set="mode=a|b|c"` steps through values.

The state lives on the nearest element you mark `data-states` (else your slot's box), as `name:value` words of that
attribute: start it with the values you want (`data-states="tab:queue"`), and follow it in CSS
(`[data-states~="tab:lyrics"] .x`). An element marked `data-show="tab:lyrics"` (or `data-show="drawer"`) shows only in
that state and is hidden otherwise, from a screen reader too. A key that sets a value or turns one on says whether it
is on: `aria-selected` inside a `role="tablist"`, else `aria-pressed`. A set value stays set when its key is pressed
again, so one tab is always open:
```html
<div data-states="tab:queue">
  <div role="tablist" aria-label="Side"><button role="tab" data-act="state" data-set="tab=queue">QUEUE</button>
    <button role="tab" data-act="state" data-set="tab=lyrics">LYRICS</button></div>
  <div data-show="tab:queue"><lull-part name="list" of="queue"></lull-part></div>
  <div data-show="tab:lyrics"><lull-part name="lyrics"></lull-part></div>
</div>
```

A part or a state is drawn only for a theme that uses one (`designs/model.js` and `designs/parts.js` load for it alone).

A template is static HTML. Two things are live:

- **Fields:** `{{name}}` becomes text, always escaped (a song title can never become markup). Use it in text and in attribute values.
- **Parts:** `<lull-part name="...">` becomes a live piece of the app: a button that already plays, a screen that updates itself. Any part takes `pick="selector"` (keep one element of it; add `inner` for only its content) and `cls="classes"` (added to its root).

Line breaks between tags are dropped and comments are removed, so indent freely; spaces within a line stay.

| Slot | Fields | Parts |
|---|---|---|
| all | `product` ("lull") | `icon` (`icon="name"`, `size="16"`) |
| `brand` | `where` (`rail`, `header`, `desktop`) | |
| `emptyArt` | `art` (`moon`, `slot`, `star`, `wave`), `title` (the state's title, e.g. "No liked songs yet"), `noun` ("liked songs" from "No liked songs yet", else empty) | |
| `homeLead` | `title`, `artist`, `album`, `trackId`, `format` ("FLAC :: 44.1 kHz, 1123 kbps"), `codec` ("FLAC :: 44.1 kHz"), `playing`, `liked`, `shuffle`, `repeat`, `repeatOn`, `repeatLabel`, `repeatIcon`, `likeClass`, `likeIcon`, `status`, `radio`, `from` (where the song plays from: "Liked songs", a playlist's name), `date`, `month`, `weekday`, `queueCount`, `queueDuration`, `queueSize`, `next`, `week` (true once Stats has numbers), `weekTime`, `weekPlays`, `weekArtists`, `weekPeak` | `cover` (`size`), `play-button` (`size`: 40, 48, 52, 56, 64), `like-button`, `track-list` (`label`), `week-clock` (24 `<i style="--v:0..1">`, the busiest `.is-peak`), `queue-sky` (the queue as `.sky-star`s joined by a `.sky-line`) |
| `nowHero` | the `homeLead` song and player fields, `error`, `variant` | `device`, `compact` (the default hero, to pick from), `screen`, `cover` (`size`), `play-button` (`size`), `like-button` (`size`), `wave` (`bars`), `facts`, `state`, `seek`, `playing-from` (the "playing from <place>" link; put it in an element with class `now-head__from`, which Now Playing refreshes) |
| `miniPlayer` | `title`, `artist`, `format`, `quality` (`ll`, `ly`), `playing`, `liked`, `playLabel`, `playIcon`, `playIconClass`, `likeIcon`, `likeClass`, `opaqueClass` | `device`, `strip`, `cover` (`size`), `wave` |

Booleans render as `true` or `false`, so CSS can follow them: `[data-playing="true"]`.

**lull's parts, in every slot.** Besides what a slot hands (the table above), every slot of lull's page takes any part of
one catalogue: `brand`, `emptyArt`, `homeLead`, `nowHero`, and the region and screen slots. Each is lull's own markup,
wired and kept current by lull exactly as lull's own copy is, so a part goes where your idea puts it and keeps working
there. It is drawn inside its `<lull-part>` element, which stays as its box with `display: contents`: the part is an
item of whatever holds it (a grid, a row), and a `display` of yours makes the box a box. `pick` and `cls` work as above.

| Part | What | Attributes | Drawn again |
|---|---|---|---|
| `play-button` | the round play key (`.btn-gloss`), Play or Pause | `size`: 40, 48, 52, 56, 64 | never: kept by `data-bind="play"` |
| `prev`, `next` | the player bar's keys | | never |
| `shuffle`, `repeat` | the player bar's toggles, pressed or not (`aria-pressed`); repeat's label and icon say the mode | | never: kept by `data-bind` |
| `like-button` | Like, for whatever plays | | never: kept by `data-bind="like"` |
| `save-button` | the player's Save, for whatever plays, with its progress | | never: kept by `data-bind="save"` |
| `seek` | the song's seek bar | `label`, `orientation="vertical"` | never: kept by `data-seek="player"` |
| `volume` | Mute and the volume slider (`.vol`) | `orientation="vertical"` | never: kept by `data-bind="mute"`, `data-seek="volume"` |
| `cover` | the playing song's cover | `size` (px, or a CSS length) | with the song |
| `source-badge` | Saved or Soulseek, a button to the source details (none for a YouTube copy, as in the bar) | | with the song and its copy |
| `quality-chip` | the copy's quality, a button to the source details | | with the song, its copy and its state |
| `playing-from` | "Playing from" the place, a link that opens it at the song | `label` (the words before the name, in every slot) | with the song and the queue |
| `facts` | the source badge, the quality chip and the stream's fact (peers, queue place, how much arrived), as Now Playing's screen has them | | with the song, its copy, its state and play or pause |
| `state` | the player's state in a few words (`buffering`, `paused`) | | the same |
| `up-next` | the next songs, as the queue's rows (a click plays from there), or "Nothing up next" with Start radio | `count` (1 to 20, 5 if absent), `label` | with the song and the queue |
| `lyrics` | the song's lyrics as Now Playing's Lyrics card draws them: synced lines that follow the song, a click to jump, text size | | with the song and when its lyrics arrive |
| `icon` | one of lull's icons | `icon`, `size="16"` | |

**An upright slider.** `orientation="vertical"` makes the `seek` or `volume` part's slider (`.seek`) run bottom to top:
it says so to a screen reader (`aria-orientation="vertical"`), a press or a drag reads the pointer's height in the bar,
and the arrow keys work as before (up and right raise it). How it looks is yours. Either turn lull's bar upright, which
keeps its fill and knob (`.vol .seek[aria-orientation="vertical"] { rotate: -90deg; }`, sized in the space you give it),
or draw your own from `--pos`, the value from 0 to 1 that lull sets on `.seek` (a fill `height: calc(var(--pos) * 100%)`
from the bottom).

A part drawn again is redrawn in place, only where it changed, so focus and what didn't change stay. In `homeLead` and
`nowHero` a part the slot hands itself is drawn as it always was (`cover`, `play-button`, `like-button`, `seek`,
`playing-from`, and the hero's `facts` and `state`); every other part comes from the catalogue. The desktop app's mini
player windows have no catalogue: `miniPlayer` takes the parts its row lists. A theme pays only for what it uses: lull
loads the catalogue, and what draws the region and screen slots (`designs/parts.js`), only for a theme (or the design it
extends) with a region or screen slot or a part only the catalogue draws where it is placed, before the page draws.

**Or no part at all.** Your own markup reaches the same features: `<button data-act="shuffle" data-bind="shuffle">SHUF</button>`
is a shuffle key lull keeps pressed (`aria-pressed`), and `<b data-bind="song.quality"></b>` the copy's quality (section
14). A button with `data-bind` lull keeps as it keeps its own, leaving your words in it: `play` (its label, and lull's icon
if you put one in), `shuffle` and `mute` (pressed), `repeat` (pressed, and its label says the mode), `like` (the playing song, and
pressed while it is liked), `save` (the playing song: pressed, its label, disabled while nothing plays), and `xfade` (shown while songs crossfade). Section 15
lists the actions a button of yours can name.

**lull never writes over your markup.** In what your slots draw, lull changes only: a binding's words (`data-bind`), an
attribute (pressed, a label, a song's id, a class lull's state gives), a custom property, what a part draws inside its
own element, and the icon you placed (its `<use>`). Your own elements and your own words stay as you wrote them, beside
lull's parts too: `<p class="now-head__from"><lull-part name="playing-from"></lull-part> FROM</p>` keeps FROM when Now
Playing draws the place again. `data-now`, `data-dk-now`, `data-dk-vol` and the other `data-dk-*` hooks are lull's own
pieces, which lull fills with its words: use one when you want lull's words there, or the live model's values with
pipes (section 14) for your own. A gate (`tests/ui/gates/slots.spec.mjs`) plays, pauses, likes, shuffles, queues and
moves between screens in a theme whose every own element says the words it holds, and fails any it finds changed.

Every slot also takes the live model's values, by path, as fields (`{{clock.time}}`, as the slot is drawn) and as bindings that stay current (`<b data-bind="clock.time"></b>`): section 14. A field is the value when the slot was drawn; a binding is the value now.

Keep the live hooks when you rebuild a hero: `data-now="title|artist|album|format|screen|c-screen|c-art|led|head|state|facts|osd"` and `data-act` / `data-bind` on transport buttons (`design/DESIGN.md` 3.5 lists them all). A custom mini player keeps `data-dk-player` and the `data-dk="..."` keys. `data-dk-now`, `data-dk-pos` and `data-dk-dur` are lull's own pieces, which lull fills with its markup and words: keep them for lull's song block, or leave them out and show the song with the live model's values (`<b data-bind="song.title"></b>`, section 14), which lull keeps current without touching your markup. The keys are `play-pause`, `previous`, `next`, `like`, `save` (saved: `aria-pressed="true"`), `shuffle`, `repeat`, `vol-up`, `vol-down`, `seek` (on the wave), `full` and `mini-close`, the names `web/desktop/mini-keys.json` lists. A key keeps your words: on `play-pause` and `like` lull points the icon in it at the one for the player's state (`<lull-part name="icon" icon="{{playIcon}}">`), and gives an empty key its icon, but never touches a key's own words (`<button data-dk="like">LIKE</button>` stays LIKE, pressed while the song is liked). The hub refuses a slot that names any other key, and says which keys there are, so a theme never ships a button that does nothing.

## 7. A full example: CSS and a slot

```
paper-moon/
  theme.json      { "id": "paper-moon", "name": "Paper moon", "api": 1, "extends": "skydeck", "shell": "rail",
                    "swatches": ["#f3efe6", "#fffdf8", "#7a5c3e", "#2b241c"], "modes": ["day"] }
  theme.css
  slots/emptyArt.html
  assets/fonts/fraunces.woff2
  assets/fonts/OFL-Fraunces.txt
```

```css
/* theme.css */
@font-face { font-family: "Fraunces"; src: url("assets/fonts/fraunces.woff2") format("woff2"); font-display: swap; }
:root { --f-ui: "Fraunces", Georgia, serif; --sky: #f3efe6; --panel: #fffdf8; --link: #7a5c3e; --fg-sky: #2b241c; --fg-panel: #2b241c; }
.card { border-radius: 2px; box-shadow: 0 1px 0 var(--line), 0 12px 24px -16px #2b241c55; }
[data-region="playerbar"] .btn-gloss { background: var(--link); }
.pm-moon { width: 96px; aspect-ratio: 1; border-radius: 50%; box-shadow: inset -18px -6px 0 var(--link); }
.pm-moon[data-art="star"] { border-radius: 30%; }
.pm-line { display: block; margin-top: var(--sp-2); font: var(--t-meta); color: var(--fg-2); text-align: center; }
.pm-line:empty { display: none; }
/* the moon dims and brightens: by opacity, 10 changes a second (section 5, Drawing cheaply) */
@media (prefers-reduced-motion: no-preference) { :root:not([data-motion="reduce"]) .pm-moon { animation: pm-glow 6s ease-in-out infinite alternate; --loop-fps: 10; } }
@keyframes pm-glow { to { opacity: .6; } }
```

```html
<!-- slots/emptyArt.html: a waxing moon, with what is missing written under it -->
<span class="pm-moon" data-art="{{art}}"></span>
<span class="pm-line">{{noun}}</span>
```

## 8. What a theme can't do

Themes never run code, and never reach outside your hub. The hub refuses a package, naming the problem, when it has:

- a `<script>`, an event handler (`onclick=`...), `<style>`, `<iframe>`, `<form>` or a `javascript:` URL in a slot, or a link that leaves the app (slot links are `#/...`);
- a URL to another site in `theme.css` (`url(https://...)`, `//host`, `@import`), or a `url()` to a file the package doesn't have; use `assets/` or `data:`;
- an SVG with a script or outside reference, or any file that isn't `theme.json`, `theme.css`, `slots/*.html`, a font, image or `.txt` in `assets/`, or a pack's files (section 13: `pack.json`, `pack.css`, `sounds/*.ogg`);
- a `theme.json` field lull doesn't know, a `window` material it doesn't know (section 12), or an `api` newer than this lull.

In the browser every template is also sanitized (DOMPurify), and the page's Content-Security-Policy only loads fonts, images and styles from the hub itself (`'self'`, `data:`, `blob:`).

## 9. Versioning

- `api` is the theme API version a theme is written for. lull speaks one version (now `1`) and refuses a theme that asks for a newer one, so update lull to use it.
- The API is: the tokens in section 4, the hooks in sections 5 and 11 (the stable ones), the slots, fields and parts in section 6, the live model's values (section 14, `web/designs/live.json`), and the package format. Adding a token, hook, field, part or slot keeps the version. Section 11's internal hooks may change in any release.
- Renaming or removing any of them, or changing what one means, is a breaking change: it raises `api`, and the release notes say what moved. A theme written for an older `api` keeps loading; whatever it relied on that moved is ignored, never an error.
- Built-in designs' own classes (`.lull-*`, `.sd-*`, `.sl-*`, `.pt-*`, `.gh-*`) are not part of the API; extend a design for its look, but target the hooks above in your own rules.

### Hooks that changed

Theme Studio (when it opens a theme, and each time it saves) and an import tell you, in plain words, when a theme styles one of these, a class or `data-*` attribute that is in neither lull nor the theme's own slots, or one of section 11's internal hooks. It's a warning: the theme still loads, and only the rules named stop doing anything.

- **2026-11-04, Like and Save in the player bar:** `.pbar [data-act="like"]` and `.pbar [data-act="save"]` moved next to the song, inside `.pbar__acts`, so a `grid-area` (or `grid-column`, `grid-row`, `order`) on them no longer places them. Place `.pbar__acts` to move both; restyle them with `.pbar__acts .btn-icon`.
- **2026-11-12, the phone's player bar:** `#mini` gained a Previous key (`[data-act="prev"]`) beside Play and Next, and its progress line (`.seek--line`) became a seek you drag (`role="slider"`, `pointer-events` on, its touch area its own box: a transparent border, `--seek-up` and `--seek-down`). The bar no longer clips (`overflow: visible`) and its end padding is `--mini-inset` on both sides; the key for another device (`[data-act="connect"]`) can give way (`[data-fit-out]`). A theme that clipped the bar, drew its line a different way, or sized the keys (`.mini .btn-icon`) under `--hit` should be checked on a phone.

## 10. A complete theme: checklist

Go through this before you share a theme, and before a built-in design ships. The five built-in designs meet all of it.

**Every screen, in every state it can be in.**
- **Home:** playing and idle (both `homeLead` variants), a first start with nothing to recommend (`.disc-cold`), loading (skeletons appear after 150 ms), a module that failed, every module empty.
- **Search:** empty with recent searches, typing, results with Soulseek still streaming in, side by side from `wide`, nothing found, Soulseek down, the Files tab, a Soulseek folder.
- **Library:** albums, artists, songs, genres and downloads, each loading, empty and with its source down.
- **Pages:** album (saved, partly saved, Soulseek only, saving, failed), artist (with and without a photo, sections still loading), tag, playlists (empty), playlist (sorted, being reordered, empty, a drop), an outside playlist.
- **Collections:** Liked, History and Following, each with its empty state.
- **Stats:** loading, empty, each period.
- **Now Playing:** every player status (`idle finding queued buffering streaming stalled fallback failed choose library paused ended jellyfin-error`), its Art, Lyrics, Queue and Details tabs on a phone, lyrics in each state (synced, plain, instrumental, not found, loading, error), the queue empty and with a radio.
- **Settings:** every section, the design picker, Your themes and Theme Studio, Classic's section.
- **The rest:** Welcome, the pairing gate and first-run setup, the Classic stage, the desktop app's mini player, strip and connect pages.

**Every region:**
- **The shell:** your shell's rail, top strip or dock; the status tray (ok, warn, down, the compact pill).
- **The side panel:** queue, lyrics and activity; docked, open over the page, collapsed to its strip, two panes.
- **The Now column:** open, and collapsed to its strip.
- **The player:** the player bar (idle, every status, crossfading, private listening, a failure in `.pbar__state`); the phone mini player and tab bar.
- **Overlays:** banners (one, several, collapsed on a phone), toasts, menus, popovers, sheets, dialogs, tooltips, the cover viewer, the source picker, the save pipeline, a drag in progress.

**Every element state:** hover, pressed (`aria-pressed`), current (`aria-current`, `aria-selected`, `aria-checked`), disabled, keyboard focus, loading, liked, playing, selected, failed.

**Every width and height:**
- phone 390×844;
- a phone held sideways, 844×390: the mini player in the player bar's place, a slim header, the page with at least half the height;
- tablet 768 and 1024;
- a short laptop, 1366×768;
- desktop 1440×900;
- `wide` 1920×1080;
- `ultra` 2560×1440.

Also a window narrowed past each `data-size` step. Nothing may scroll sideways, and no title may be squeezed.

**The notch and the home indicator:** lull keeps `.app` inside the safe-area insets at a phone's sides, and from 640 px
above and below too, with margins, so your padding on `.app` keeps them. Under 640 px the phone's header, tab bar and
Now Playing's sheet pad into the top and bottom insets themselves, so their paper reaches the screen's edge: a theme that
sets its own padding there adds the inset (`calc(8px + env(safe-area-inset-top))`), or moves a floating bar clear of it
with its margin. Nothing to read or tap goes under an inset.

**A thumb's reach:** on a touch screen (`@media (pointer: coarse)`) lull gives `.btn-icon`, `.tile__play`, the compact
status tray, the d-pad's keys and the phone bar's cover (`.mini .bezel-thumb`) an invisible `::before` that reaches `--hit` (44 px) round their centre, whatever size
they are drawn at; leave that `::before` free on them, and keep such a key positioned (`position: relative`) so it is
the key's own. A key of your own drawn smaller does the same, and keys in a row stand 44 px apart, centre to centre, so
each touch area is its own.

**Hover is for a pointer that hovers:** a phone keeps `:hover` on what was tapped until the next tap elsewhere, so
lull's own hover styles sit in `@media (hover: hover)`; put yours there too, and give what a hover reveals another way
in (`:focus-within`, or drawn without one).

**Every mode** the manifest lists, with each token you set given in both the day and the night block.

**Contrast, information for you:** lull reports where text may be hard to read, so you can weigh it. It is never a rule:
a theme that matches its design and falls short of a ratio is not wrong, and nothing in lull asks you to change your
look for a check (ADR-0101).
- **The ratios it reads against:** text 4.5:1 and large text 3:1; outlines of controls and the parts that show state (a pressed toggle, a knob, a focus ring) 3:1 against the surface under them. Text on a gradient or glass is checked against its darkest and its lightest stop.
- **In every state:** check them in each of the states above, hover included, in both modes. A hover that only changes colour must still pass.
- **Theme Studio** warns for its token pairs; anything else you draw is yours to check.
- **Never colour alone:** where a song comes from (saved or Soulseek) and every service state keep their shape and icon, never colour alone.

**Motion:**
- **Cheap to run:** animate `transform` and `opacity` only, so it runs off the main thread on a slow phone.
- **Cheap to draw:** each loop is stepped where it is declared (`steps(n)` or a `--loop-*` stepped easing), moves only what moves, and stays off live glass; glass over your own wallpaper is baked (section 5, Drawing cheaply).
- **As much as you like, within its cost:** how much moves is yours to choose; the bound is that it stays smooth on a cheap device (the checks above, and the perf gates for the built-in designs).
- **Off under reduced motion:** nothing moves under `:root[data-motion="reduce"]`, or under the system's reduced-motion setting unless `data-motion="full"`. lull's base CSS already stops animations and travel there; don't undo it with `!important`.
- **Off on the processor:** under `:root[data-graphics="light"]` (the desktop app on a computer whose graphics card isn't drawing the page) nothing moves and nothing blurs what's behind it, whatever the motion setting. Glass there shows its fill alone, so give it a fill that text reads on.
- **Paused when hidden:** loops pause on `:root.is-hidden` (the page is hidden).
- **Still when asked:** a decorative loop takes `animation-play-state: var(--loop-play, running)`, which pauses on `:root.is-still`, or names its rate with `--loop-fps`, which lull holds; a progress loop leaves the first out, or sets `--loop-holds: no` (section 5, Drawing cheaply).
- **Keep restyles small:** follow "Staying fast" in section 5.

**Shared behaviour stays:**
- **Accessibility:** keep every control visible and reachable, with its accessible name.
- **Slots:** keep the live hooks a slot needs (section 6).
- **Layout lull controls:** leave `.seg__ind`'s position and the dragged widths (`--ctx`, `--now-w`) alone, and style the layout states that are `!important` rather than fight them.

**Preview it:**
- **A listener's theme:** put the folder in `<data dir>/themes/<id>/` on a hub, or drop the `.lulltheme` on Settings › Appearance. Open devices reload it within a second of each save. `mise run dev` runs a hub on your machine (settings in `.env`).
- **A built-in design:** `node tests/ui/fixture-hub/server.mjs 8790` serves your checkout's `web/` at `http://localhost:8790` with an invented library (150 albums, a 1,200-song playlist, lyrics) and no outside services.
- **Hash flags:** pick a design and theme for one visit with `#/home?design=<name>&theme=night` (also `motion=reduce`).
- **Against your own hub:** serve your checkout's `web/` files over the hub's (for example with Playwright's request routing) to see your own library.
- **Every screen at once:** `mise run themes:check -- --mine home now mini`, with `LULL_THEMES_DIR` set to the folder that holds your theme folders. It draws each screen with a song playing, on desktop and phone, day and night, and writes one contact sheet, `tests/ui/themes-check/index.html`. It works on a copy, so nothing in your folder is changed.
  - **Which looks:** `--mine` draws every theme in `LULL_THEMES_DIR` and none of the built-ins. `--only probe,lull` draws only the looks you name, yours or built in. With neither, it draws all of them.
  - **Which sizes:** `--sizes 1280x800,1440x900,1920x1080` draws the desktop at each size in one run. `--size 1536x864` draws one. The phone is always 390×844.
  - **The mini player:** the route `mini` draws the desktop app's mini player windows, the device and the strip, in a plain browser. They open at the app's width and fit their height to the player, as in the app. Your theme reaches them as one package, as it does in the app. Where the window is see-through, the sheet shows a checkerboard.
  - **Beside your mockup:** `--mockup "C:/mockups/my-mockup.html?mode={mode}&screen={route}"` draws the mockup next to each picture, at the same size. `{mode}` becomes `day` or `night`, `{route}` the route's name (`home`, `mini`, `settings-appearance`), and `{size}` one of `desktop`, `phone`, `device` or `strip`. A file with no placeholders is shown as it is. A path, a `file://` URL or an `http://` URL all work.
  - **Where the sheet goes:** `--out runs/today` writes the contact sheet and its pictures to that folder instead, from the folder you run it in (or give a full path), so two runs from one checkout keep both sheets. Each run empties the folder first. A folder that holds anything other than an earlier sheet is refused.

**Run the checks:**
- **`mise run check`** includes this document's own gate (`tests/web/theme-hooks.test.mjs`): every hook lull's CSS and markup use must be in section 11, and a built-in design may not style a hook that no longer exists.
- **`mise run check:ui`** runs the UI gates on every built-in design: performance budgets, the key screens and accessibility (docs/context/quality.md#ui-gates).
- **`mise run themes:check`** (see Preview it) runs these checks on every picture and lists what they find under the picture in the contact sheet. The checks of function (fit, every feature reachable) are findings, and the run exits 1 for them. Contrast is information, shown in amber, and never counts.
  - **Scrollbars as the app shows them:** the desktop pictures and the mini player show scrollbars wherever something scrolls, as the desktop app always does, so you see the ones your theme left to lull. A scrollbar your CSS never styled is named in amber, also as information that never counts: a `::-webkit-scrollbar` rule (or one of its parts) that reaches the element, `scrollbar-color` or `scrollbar-width` on it, or `--scroll-thumb` set on it or round it all count as styled. The phone shows none: a phone's scrollbars appear only while scrolling.
  - **Fit:** a control nothing can click, text cut off or on other text or under something that paints over it, a feature nothing reaches (section 5), anything sticking out sideways, a squeezed title, and console errors. An element faded out (opacity 0, its own or an ancestor's) or `visibility: hidden` paints nothing, so text under it is fine.
  - **Every feature one click away:** after each picture the menus and sheets one click opens there (the player's "…", the phone's More sheet, Following and playlists) are opened, and each must hold what `web/designs/features.json` says it does, reached by a click inside it.
  - **Contrast against what is drawn (information):** each line of text is read against the pixels actually behind its letters: the picture, gradient or glass, not the token it was meant to sit on. A line is reported when 5% of those pixels give under 4.5:1, or 3:1 for large text (24px, or 18.66px bold). Punctuation is read by its own ink, wherever it sits in the line. The sheet names the worst line, its ratio and its colour, and how many more fall short.
  - **What the contrast check counts:** an outline (`-webkit-text-stroke`) counts as ink, and a fill counts against an outline 2px or wider. A line hidden under a fade laid over it is left out, and so is a line where a mask on it, or round it, fades it (a list dissolving at its scroller's edge). Text over a masked shape of yours (fog, a feathered patch) is judged against that shape as drawn. A text's own shadow counts as what is behind its letters, as drawn: a dense shadow close to the letters backs them (white on a pale sky with `0 0 1px`, `0 0 3px` and `0 0 6px` of a dark colour reads), a faint one doesn't. The check reads what is behind the letters themselves, not the gaps between them. A see-through mini player is checked over black and over white.
  - **What it skips as decoration:** text drawn at under 50% opacity (its own times its ancestors'), and text blurred by a `filter` on it or round it. A blurred, faded copy of a title behind it is not checked; the title itself is. Text hidden from screen readers (`aria-hidden="true"`) is still checked when it shows: a sighted listener reads it.
  - **What it can't check:** text in form fields, SVG and `::before`/`::after`, and text filled with a picture (`background-clip: text`). 
  - **Advisory, for every theme:** contrast is shown in amber with the line and its ratio, for you to weigh, and never counts towards the exit code, on the built-in designs and on yours alike. Build your design as it is drawn; change it for a ratio only if you agree with it.
- **Visual goldens:** after a visible change, push and run `mise run ui:approve` to recapture the gates' goldens. `mise run ui:update` recaptures the full visual matrix. Review the pictures and commit them with the change.
- **A new built-in design** also joins the lists of built-ins:
  - `D.builtIn` in `web/designs/designs.js`;
  - `BUILT_IN` in `crates/themes/src/package.rs`;
  - `designs` in `tests/ui/routes.mjs` and `tests/ui/gates/lib.mjs`;
  - `DESIGNS` and its own class prefix in `tests/web/theme-hooks.test.mjs`.

  Then run `LULL_WRITE_PACKAGES=1 cargo test -p lull-themes` to write its package script, and add its picker previews.

## 11. Hook reference

Every class lull's CSS styles and every `data-*` attribute lull's markup sets, each stable or internal. `mise run check` fails when one is missing here, so this list is complete. A class stands for its BEM block: `.trow` covers `.trow__num` and `.trow--dense`. State classes (`.is-*`, `.has-*`) stand alone and mean the same wherever they appear.

**Stable** hooks are the API (section 9): style them freely. **Internal** ones (at the end) are lull's plumbing and may change in any release: a theme that styles them may break.

### Root and shell

- **Root:**
  - `data-design`, `data-shell`, `data-theme` (`day` `night`), `data-theme-pref` (`day` `night` `system`), `data-motion` (`reduce` `full`);
  - `data-classic` (`on` while the Classic stage is open);
  - `data-theme-fade` (set for a moment while the listener flips day and night, so a design can animate the change);
  - `data-titlebar` and `data-window-inactive` (the desktop app);
  - `data-player` (the player's state, as `#pbar[data-status]`) and `data-playing` (`true` while sound plays);
  - `data-window` (`mica` `acrylic` `blur` `transparent`: the desktop app's window shows the desktop behind the page, section 12; absent while it is solid) and `data-window-preview` (the browser's preview of it, over a black and white checkerboard);
  - `data-graphics` (`light`: the desktop app's lighter look, where the processor draws the page: no movement, no blur behind glass);
  - `.is-hidden` on `:root` (the page is hidden: loops pause);
  - `.is-still` on `:root` (decorative loops hold still: the listener's "Pause animations when lull isn't the active window", section 5, Drawing cheaply);
  - the live model's states and flags a theme names, by path with dashes (section 14): `data-clock-*`;
  - `--loop-<curve>-<n>` stepped easings and `--loop-play` (`paused` while `:root.is-still`) for a loop's timing and play-state; `--loop-fps` (a loop's changes a second, lull steps it) and `--loop-holds: no` (it shows progress) on a loop that names its rate instead (section 5, Drawing cheaply).
- **The app:**
  - `.app` with `data-screen`, `data-route`, `data-view`, `data-ctx`, `data-size`, `data-height`, `data-window-controls` (`own`, by the switch `--layout-window-controls`);
  - `data-panel` (`strip`), `data-panel-split` and `data-two` (Queue and Lyrics as two panes), `data-nowcol` (`open` `strip`), `data-nowcol-ok`;
  - `.is-resizing` (a resize handle is being dragged);
  - the layout switches, `--layout-*` (section 5, Layout switches): custom properties a theme sets on `.app`, which lull's script reads.
- **Regions:** `data-region` (section 5), `.region-title`, `.region-meta`.
- **A theme's slots at the edges** (section 6): `.region-slot`, the box a `before` or `after` slot is drawn in, the first or last child of its region or screen, with `data-slot` naming it (`after.playerbar`, `before.home`). A part of lull's catalogue keeps its `lull-part` element as its box (`display: contents`).
- **Live hosts:** `data-live` on an element of a theme's slot markup: it carries the live model's custom properties, as lull's own hosts do (section 14).
- **State a theme keeps** (section 6, State): `data-states` on an element of a theme's markup holds its state as `name:value` words (`[data-states~="tab:lyrics"]`); `data-set` on a `data-act="state"` key says what it sets; `data-show` on an element shows it only in a state. A list's own row is `lull-row`, its drawn rows `.lull-list__row` in `.lull-list` (style a row by its item with your own markup: `class="row--{{item.kind}}"`).
- **Shell parts:** `.rail`, `.header`, `.main`, `.view` (with `--view-h` once Settings has opened: its height below its top padding, where something sticky at its top ends), `.ctx` (with `data-ctx-tab` on its tab buttons), `.ctx-scrim`, `.nowcol`, `.pbar`, `.mini`, `.tabbar`, `.tab`, `.nav-item`, `.nav-pill`, `.wordmark`, `.brand` (the default brand slot), `.skip-link`, `.sky-specks`, `.col-resize` (a resize handle, `data-resize`).
- **Links:** `data-nav` on every nav link, `data-brand` on brand links.
- **Rows that fit themselves** (section 5, Wide screens): `data-fit-row` on a row (the header, the player bar, its tools,
  the song's chips), `data-fit` on a part that can give way (its value is its order, 1 first; a theme's `--fit-order`
  replaces it, and `--fit-sides: none` on a row ends its mirrored sides: section 5, Layout switches), `data-fit-out` on a part
  that gave way, `data-fit-gave` on the row (the orders that gave way, and `all` once every one did); `data-fit-sides`
  on a row in three columns whose sides lull sizes alike (the player bar, `--fit-side`), from its `data-fit-side` row
  (the tools); `data-fit-keep` on words that keep a readable width (the song's title and artist, at least `--fit-keep`
  or all of them), which the row's parts give way to protect. Style how a part
  that gave way looks (a nav pill as its icon); lull's own CSS already takes away what must go, and puts it in the "…"
  menu. To style something else by what gave way, read the row (`.header[data-fit-gave~="2"] …`), not
  `:has([data-fit-out])`, which restyles the whole page at every step of a fit.
- **Status tray:** `.tray` with `.tray__pill[data-svc][data-state]`, `data-worst`, `data-cache-used`, `data-cache-cap`, `data-tray`; `data-activity-count` holds the number of running jobs.
- **Banners:** `.banner`, `.banner-slot`, `.banner-more`, `.is-lead` (the banner a phone shows when the others collapse).
- **Surfaces:** `.s-sky`, `.s-panel`, `.s-card`, `.s-screen`.

### Components

- **Buttons:** `.btn`, `.btn-icon`, `.btn-gloss`, `.btn-save` (`data-state`), `.kbd`, `.switch`, `.switch-row`.
- **Every control:** `data-act` names its action (section 15 lists those a theme's own button can name), `data-tip` its tooltip (with `data-tip-cut`, words that show it only while they are cut off, and on a tap: the state tab's line, the mini player's; a tooltip a tap showed is `.tooltip--asked`, which a theme that hides tooltips on phones should still show), `data-bind` the player state it shows (`play`, `shuffle`, `repeat`, `like`, `save`, `mute`, `xfade`, `private`, and the times `pos`, `dur`, `buffered-to`, `received`: section 6), or the live model's value it holds (section 14). `data-answer` (`yes`, `no`) is the player bar's "Right song?" answer (`.pbar__ask`, internal).
- **Chips and badges:** `.chip`, `.chip-reason`, `.chip-more`, `.pill-filter`, `.badge`, `.badge-saved`, `.badge-slsk`, `.badge-new`, `.badge-count`, `.badge-below`, `.q-chip`, `.src-badge` (`data-was`: `saved` `slsk`), `.dup-note`, `.dup-better`, `.resolve`, `.scrobble`, `.sg`, `.spin-ring`.
- **Cards:** `.card` (`data-head`: `rule` `hidden`; `data-meta`), `.card-head`, `.label-rule`, `.inset`, `.source-card`, `.disc-pick` (a Home pick).
- **Covers:** `.cover`, `.c1`, `.c2`, `.c3`, `.c4`, `.c5`, `.c6`, `.c7`, `.c8`, `.has-art`, `data-look` (cover variety), `.mosaic`, `.bezel-thumb`, `.cover-zoom`, `.cover-view`.
- **Tiles and lists:**
  - `.shelf`, `.tile`, `.tlist`, `.trow`;
  - row facts `data-khz`, `data-kbps`, `data-bits`, `data-size`;
  - `.eq`, `.place-mark`, `data-place`, `.from`;
  - `.dl-mark` and `.dl-btn` (downloads; `data-on` on a downloaded row's mark);
  - `data-sortable` (a queue row that can be dragged);
  - `.drag-chip`, `.drop-line`, `.drop-overlay`, `data-drop`.
- **Player parts:** `.seek` (`data-seek`, `data-pointer`), `.seek-row`, `.wave`, `.wave-times`, `.vol`, `.range`, `.progress`, `.pipeline`, `.pipeline-wrap`, `.device`, `.dpad`.
- **Source picker:** `.src-table`, `.src-row`, `.score`, `.src-speed` (`--said`: a speed the peer reports, not one lull measured).
- **Inputs:** `.input`, `.field`, `.search-field`, `.seg`.
- **Overlays:** `.sheet`, `.sheet-bar`, `.dialog`, `.scrim`, `.popover`, `.menu`, `.menu-list`, `.tooltip`, `.toast`, `.toast-region`, `.shelf-pop` (the rail's tags and playlists on a tablet), `.shortcut-list`, `.shortcut-row`.
- **Status and empty states:** `.status-list`, `.status-row`, `.empty` (with `data-art` and `data-title` in its art; `data-noun` in slot templates), `.skel`, `.skel-row`, `.skel-tile`, `.skel-group`, `.sparkle`.
- **Type and icons:** `.t-display`, `.t-title-xl`, `.t-h2`, `.t-row`, `.t-body`, `.t-meta`, `.t-micro`, `.t-px`, `.t-lyric`, `.px`, `.link`, `.link-plain`, `.ic`, `.ic-16`, `.ic-24`.

### States

- **Playback:**
  - `.is-playing` (the row or place playing), `.is-playing-now` (and sound is coming out), `.is-playing-place`, `.is-revealed`;
  - `.is-current`, `.is-past` (a lyric line; `.is-current` also marks a pipeline step);
  - `.is-buffering`, `.is-transferring`, `.is-played`, `.is-buf`, `.is-xfer` (seek and wave parts);
  - `.is-scrubbing`; `.is-going` (a seek kept until its part arrives: `.seek__target` marks where it lands, at `--target`);
  - `.is-power-on` (a device's power-on moment as the song starts);
  - `.is-peak` (the busiest hour in the `week-clock` part);
  - `.sky-star` and `.sky-line` (the `queue-sky` part).
- **Status:**
  - `.is-done`, `.is-failed`, `.is-skipped`, `.is-error`, `.is-below` (below the minimum bitrate), `.is-preferred`, `.is-unavailable`;
  - `.is-liked`, `.is-selected`, `.is-on`, `.is-open`, `.is-wide` (the player bar's view tools: Lyrics, Classic, Device view), `.is-dragging`, `.is-drop-target`;
  - `.is-applied`, `.is-bad` (Classic skins);
  - `.is-out` (Now Playing's slide while the song changes).
- **Player status:** `data-status` (`#pbar`, `#mini`, the mini player windows' `[data-dk-player]`, an album's save status), `data-status-text` (`#pbar`, `#mini`, `[data-dk-player]`, with `--status-text`), `data-playing` (also on `:root`, with `data-player`), `data-state`, `data-liked` and `data-saved` (the playing song's, on `#pbar`, `#mini`, `.now__dev`, `.now__devc`); `data-better-copy` (`true` on `#pbar` and `#mini` while the song plays a better copy it switched to mid-song; the player bar's quality chip then carries `.q-chip--better`, a small arrow before its words); `--song-pos` (the song's place in hundredths, on the parts section 5 names).

### Home and Search

- **Home:**
  - `.disc-home`, `.disc-hello` (the greeting band), `.disc-lead` (the `homeLead` slot);
  - modules: `.disc-modules`, `.disc-mod` (`data-module`), `.disc-card`;
  - their contents: `.disc-picks`, `.disc-shelf`, `.disc-rel`, `.disc-starters`, `.disc-starter`, `.disc-jump`, `.disc-tags`, `.disc-tag`;
  - where a pick can play from: `[data-avail]` on `.disc-pick` (`soulseek`, `youtube`, or both, space-separated; absent until lull knows) and its mark `.pick-avail`, one `.pick-avail__src` per source (`data-src`: `soulseek` `youtube`). Quiet by default (14 px glyphs in `--fg-3` at the end of the pick's reason row, `.disc-pick__why`). The mark's markup comes with the first availability answer (`screens/avail.js`), so for a moment after Home draws a pick may carry `[data-avail]` before its mark; restyle it, move it with `order`, or hide it (`display: none`). A pick no source has shows "Search again";
  - loading and empty: `.disc-skel-shelf`, `.disc-picks-skel`, `.disc-cold`, `.disc-empty`.
- **Search:**
  - the page: `.disc-search`, `.disc-q` (the search field on phones), `.disc-tabs`, `.disc-filters`, `.disc-recents`, `.disc-recent`;
  - results: `.disc-duo` (catalog and Soulseek), `.disc-splitview` (side by side) and its switch `.disc-split`, `.disc-top` (the top result), `.disc-songs`, `.disc-scan` (Soulseek's live count);
  - Soulseek: `.disc-folder` and `.disc-folder-tile` (a Soulseek folder), `.disc-dup` (a saved copy beside a Soulseek one), `.disc-kv` (source details);
  - sheets: `.disc-src` (the source picker sheet), `.disc-step` (a save step).

### Now Playing

- **The page:** `.now` (`data-tab`: `art` `lyrics` `queue` `details` on a phone; `data-page`: `now` `source` `save`), `data-sec` (which tab a section belongs to).
- **The hero:** `.now-head`, `.now-acts`, `.now-dev` and `.now-devc` (the hero and its compact form), `data-now` (live parts of a hero, section 6), `data-now-cover` (the hero's cover part, with the size it asked for), `data-like`, `.now-slide`, `.now-osd`, `.now-sbtn` (a button on the device screen).
- **The cards:** `.now-ly` and `.now-lycard` (lyrics), `.now-q` and `.now-qcard` (the queue), `.now-next`, `.now-src` and `.now-srccard` (the source), `.now-seg` (the phone tabs).
- **Pairs a design may lay out as one box:** `.now__intro` (`.now-head` and the actions after it) and `.now__side` (`.now-srccard` and `.now-lycard`). Each is `display: contents` until a design gives it a box, so the parts stay items of the page's grid; give it one to place the pair together and let the layout space them (lull stacks the lyrics over the source in `.now__side`; petal writes the title on the page with its actions under it in `.now__intro`), never fixed heights or offsets guessed from the other's size.

### Library and pages

- **Library:**
  - page structure: `.lib`, `.lib-head`, `.lib-kicker`, `.lib-segs` (the sub-nav), `.lib-count`, `.lib-page`;
  - grids and loading: `.lib-grid`, `.lib-tile`, `.lib-newtile`, `.lib-skel-shelf`;
  - list parts: `.lib-listhead`, `.lib-notice`, `.lib-rule`, `.lib-cols`, `.lib-card`, `.lib-actions`, `.lib-bar`;
  - history: `.lib-day` (a day in History), `.lib-history`.
- **Album:** `.lib-hero` (also tag and playlist heads; `data-tracks`, `data-duration` and `data-size` carry the album's facts), `.lib-bio`, `.lib-disc` (a disc heading).
- **Artist:** `.lib-artist`, `.lib-ahero`, `.lib-popular`, `.lib-about`, `.lib-disco` (the discography).
- **Playlist:** `.lib-pl`, `.lib-pl-list`, `.lib-thead`, `.lib-th`, `.lib-table`, `.lib-sort`, `.lib-sortnote`, `.lib-site` (a bookmark's site, `data-site`).
- **The add-to-playlist picker:** `.lib-picker`, `.lib-pick`, `.lib-pick-list`, `.lib-pick-new`, `.lib-pick-none`.

### Settings, Stats and first run

- **Settings:**
  - structure: `.sys-settings`, `.sys-nav` (with `--nav-lift` beside a section: how far below where it sticks it still sits, the page's heading's height until the page scrolls past it; lull's list is `--view-h` less that tall), `.sys-page`, `.sys-section`, `.sys-card`, `.sys-group`, `.sys-row`, `.sys-grid`, `.sys-span`, `.sys-head`, `.sys-lede`;
  - controls: `.sys-period`, `.sys-pal`, `.sys-usage`, `.sys-ios`;
  - the design picker: `.sys-designs`, `.sys-design` (`data-value` is the design's id; on other elements `data-value` is wiring).
- **Stats:** `.sys-stats`, `.sys-tiles`, `.sys-tile`, `.sys-bars`, `.sys-bar`, `.sys-heat` (`data-l`: the cell's level, 0 to 4), `.sys-empty-stats`.
- **First run and pairing:** `.sys-welcome`, `.sys-gate` (the pairing gate), `.setup`, `.setup-step`, `.setup-code`, `.sys-step`, `.hub-pair` and `.hub-qr` (the pair dialog).
- **Connect:** `.cx-pop` (the device picker), `.cx-dev`, `.cx-remote`.
- **Equaliser:** `.eqp` (the panel: `.eqp--full` in Settings > Playback, `.is-off` while it is off), `.eq-pop` (the player's popover), `.eq-sheet` (the same on phones). Its sliders are `.range.range--centred` (filled from 0 dB).

### Classic and the desktop app

- **Classic:**
  - the stage: `.cls-desk`, `.cls-scrim`, `.cls-back`, `.cls-busy`, `.cls-failed`;
  - Classic's section in Settings: `.cls-now`, `.cls-browser`, `.cls-grid`, `.cls-card`, `.cls-shot`, `.cls-skel-shot`, `.cls-pop`, `.cls-look`, `.cls-nsfw`, `.cls-note`, `.cls-meta`, `.cls-attrib`, `.cls-more`, `.cls-folder`, `.cls-skipped`.
- **Easter eggs** (section 13): `data-egg` on `:root` (the live eggs' names), `data-egg-word`, `--egg-count`; `.egg-stage` (the layer an egg draws in) with `.egg-line` (its words) and `[data-egg-of]` (an egg slot's box).
- **Desktop app pages:**
  - the mini windows: `.dk-mini`, `.dk-strip`, `.dk-strip-wrap`, `.dk-art`;
  - window chrome: `.dk-titlebar`, `.dk-caption`, `.dk-glyph`, `.dk-tip`;
  - pages and dialogs: `.dk-page`, `.dk-card`, `.dk-form`, `.dk-hubs`, `.dk-status`, `.dk-offline`, `.dk-retry`, `.dk-modal`;
  - the mini player's keys (section 6): `data-dk`, `data-dk-player`, `data-dk-now`, `data-dk-pos`, `data-dk-dur`, `data-dk-buf`, `data-dk-vol`, `data-dk-title`, `data-dk-artist`, `data-dk-thumb`, `data-dk-play-label`, `data-tauri-drag-region`.

### Internal

lull's plumbing: these may change in any release.

- **Built-in designs' own:** `.lull-*`, `.sd-*`, `.sl-*`, `.pt-*`, `.gh-*` classes, and the attributes only their slots set: `data-p`, `data-week`, `data-track`.
- **Layout utilities:** `.cluster`, `.flow`, `.stacked`, `.spacer`, `.ellipsis`, `.vh`, `.num`, `.prose`, `.fg-2`.
- **Boot:** `data-booting`.
- **A list part's rows:** `data-entry`, what the list adds to a row's item (its queue place, a play's time), for the parts in the row.
- **Home and Search parts:** `.disc-agg`, `.disc-aggl`, `.disc-alt`, `.disc-alts`, `.disc-cat`, `.disc-cloud`, `.disc-col`, `.disc-fallback`, `.disc-issue`, `.disc-more`, `.disc-note`, `.disc-pending`, `.disc-picker`, `.disc-recover`, `.disc-shead`, `.disc-sheet`, `.disc-steps`, `.disc-sugg`, `.disc-tile-open`, `.disc-why`, `.disc-why-pop`, `.is-all`, `.is-better`, `.is-down`, `.is-nosource`, `.is-stale`, `.is-wait`.
- **Now Playing parts:** `.now-act`, `.now-fade`, `.is-in`, `.is-lifted`, `.is-ready`, `.is-sorting`.
- **Library parts:** `.lib-adddesc`, `.lib-chips`, `.lib-cover`, `.lib-dl-storage`, `.lib-dropzone`, `.lib-facts`, `.lib-flist`, `.lib-foot`, `.lib-form`, `.lib-frow`, `.lib-help`, `.lib-hist`, `.lib-imp`, `.lib-import`, `.lib-kept`, `.lib-legend`, `.lib-link`, `.lib-link-card`, `.lib-linkrev`, `.lib-m3u`, `.lib-more`, `.lib-out`, `.lib-status`, `.lib-suggest`, `.lib-tagcard`, `.lib-tags`, `.lib-textarea`, `.lib-title`, `.lib-toggle`.
- **Settings and hub parts:**
  - Settings and Stats: `.sys-back`, `.sys-chips`, `.sys-combo`, `.sys-detail`, `.sys-follows`, `.sys-formats`, `.sys-funnel`, `.sys-health`, `.sys-hit`, `.sys-inline`, `.sys-keys`, `.sys-legend`, `.sys-locked`, `.sys-pending`, `.sys-meter-label`, `.sys-newart`, `.sys-nomatch`, `.sys-off`, `.sys-palette`, `.sys-plus`, `.sys-presets`, `.sys-preview`, `.sys-range`, `.sys-rescan`, `.sys-seg` (with `data-cols`, `2` or `1`, while its choices wrap to fit their row), `.sys-status`, `.sys-stepper`, `.sys-steps`, `.sys-taste`, `.sys-then`, `.sys-version`, `.sys-themes`, `.sys-packs`, `.sys-surprises`;
  - hub and Connect: `.sys-cfg`, `.sys-connect`, `.sys-note`, `.hub-cred`, `.hub-dev`, `.hub-devs`, `.hub-form`, `.hub-jf`, `.hub-lib`, `.hub-qc`, `.hub-slsk`, `.cx-foot`, `.cx-list`, `.cx-note`, `.cx-title`;
  - states: `.has-section`, `.has-shot`, `.is-disabled`, `.is-index`, `.is-here`, `.is-online`;
  - Theme Studio: `.studio`, `.studio-sheet`, `.is-drop`, `.is-phone`.
- **Classic and desktop parts:** `.cls-root`, `.cls-slot`, `.cls-frame`, `.is-closed`, `.is-leaving`, `.dk-actions`, `.dk-choice`, `.dk-choices`, `.dk-code`, `.dk-details`, `.dk-dragstrip`, `.dk-hb`, `.dk-kv`, `.dk-sep`.
- **Attributes that carry ids and wiring, never looks:**
  - ids and keys: `data-id`, `data-ids`, `data-key`, `data-fk`, `data-uid`, `data-track-id`, `data-album-id`, `data-pl`, `data-qi`, `data-place-at`, `data-dl-mark`, `data-queue-index`, `data-si`, `data-pi`, `data-pic`, `data-wait-*`, `data-want-photo`, `data-i`, `data-t`, `data-d`, `data-q`, `data-ref`;
  - values and wiring: `data-kind`, `data-name`, `data-label`, `data-dir`, `data-pos`, `data-group`, `data-item`, `data-via`, `data-what`, `data-job`, `data-mode`, `data-choice`, `data-close`, `data-yes`, `data-form`, `data-url`, `data-text`, `data-why-text` (the why popover's sentence, filled in once `screens/why.js` arrives), `data-msg`, `data-locked-msg`, `data-placeholder`, `data-cmd`, `data-device`, `data-remote`;
  - lists, filters and banners: `data-filters`, `data-issue`, `data-more`, `data-out`, `data-period`, `data-range`, `data-pick`, `data-pick-new`, `data-pick-made`, `data-pick-again`, `data-pick-done`, `data-sync`, `data-save-btn`, `data-save-songs`, `data-src-badge`, `data-src-mini`, `data-dl`, `data-cold-count`, `data-banner`, `data-banner-*`, `data-toast-*`, `data-cover-*`, `data-scan*`, `data-now-qlist`, `data-now-steer`, `data-fill` (a long list's row whose contents are drawn only near the screen: empty `<li>` while far from it);
  - Settings controls: `data-cfg`, `data-switch`, `data-step`, `data-stepper`, `data-min`, `data-max`, `data-unit`, `data-weight`, `data-when`, `data-format`, `data-formats`, `data-preset`, `data-presets`;
  - screens with their own wiring: `data-pack`, `data-imp-*`, `data-lib-*`, `data-linkrev*`, `data-rev-*`, `data-cx*`, `data-studio*`, `data-drop-theme`, `data-theme-file`, `data-add-rates`, `data-skin`, `data-skin-file`, `data-webamp`, `data-eq*`, `data-token`, `data-token-reset`;
  - the desktop app's pages: `data-attempt`, `data-dk-attempt`, `data-dk-close`, `data-dk-connect`, `data-dk-error`, `data-dk-found`, `data-dk-look` (Settings › App: the other look, desktop.js), `data-dk-own-helpers` (Settings › App: lull's own helper copies, desktop.js), `data-dk-offline`, `data-dk-retry-status`, `data-dk-welcome`.

## 12. A see-through window

In the desktop app on Windows and Linux, a theme can show the real desktop through lull's window, with the system's own glass behind it. Everywhere else the window stays solid and the theme looks as it always does, wallpaper included: in the browser, on Android, on a computer that can't draw glass, and for a listener who turned it off.

**Asking for it.** One line in theme.json:

```json
"window": "glass"
```

That is frosted glass where this computer can draw it, tinted with your theme's own page colour. The full form is a small object; `day` and `night` override the rest for that mode:

```json
"window": {
  "material": ["acrylic", "blur"],
  "tint": "#f4f1ff",
  "opacity": 0.6,
  "night": { "material": "mica", "tint": "#14101f", "opacity": 0.7 }
}
```

| Field | What |
|---|---|
| `material` | What shows behind the window: a material, or a list in your order of preference (below). Default: `"glass"`. |
| `tint` | `#rrggbb` that lull lays over the desktop. Default: your `--sky`. |
| `opacity` | How much of the tint covers the desktop: 0 is clear glass, 1 a solid tint. Default: 0.85. |
| `day`, `night` | `material`, `tint` and `opacity` for one mode. |

| Material | What it is | Where |
|---|---|---|
| `mica` | The listener's wallpaper, blurred and tinted by the system; other windows don't show through | Windows 11 |
| `acrylic` | Frosted glass over whatever is behind the window | Windows 11 22H2 and later |
| `blur` | Plain blur over whatever is behind the window | Windows 10; KDE Plasma on X11 |
| `transparent` | Clear: the desktop as it is | Windows; Linux with a compositor |
| `none` | Solid. In a list it ends the list | everywhere |

**The fallback chain.** lull uses the first material in your list that this computer draws, and keeps the window solid when it can draw none of them. A single word means that material and the gentler ones after it in `mica`, `acrylic`, `blur`, so `"glass"` and `"mica"` both mean all three. `transparent` is used only when you list it. A material that makes the window lag is never used: Windows 10 draws acrylic, and Windows 11 draws plain blur, through an old path that makes the window trail the pointer when it is dragged (docs/context/desktop.md), so those count as not there. Asking for `glass` gets Mica on Windows 11, blur on Windows 10 and KDE on X11, and a solid window elsewhere.

The hub checks the field when you install a theme and refuses one it doesn't know by name (`window.material "frosty" is not one of mica, acrylic, blur, transparent, none`).

**What the page gets.**
- `:root[data-window]` names what the window shows *now*. It may be less than you asked for, so style for the material you got, never the one you asked for. Absent, the window is solid. A listener's **See-through window** switch (Settings › Appearance), Windows' **Transparency effects** and the system's reduced transparency (`prefers-reduced-transparency`) all turn it off, and then it is absent too.
- While it is set, lull lays `--window-backdrop` over the desktop where your wallpaper was: it clears the page's own background and hides the sky specks. Nothing else changes. Every part your theme paints stays as you paint it, so a part shows the desktop exactly where you make it transparent or translucent, all the way down.
- `--window-backdrop` is the tint at its opacity (`--window-tint` and `--window-opacity`, from theme.json). Set it yourself for anything else: a gradient, or `transparent`.
- Switching to or from a glass theme changes the window at once on Windows. On Linux a window gets the channel that makes it see-through only when it opens: a theme that turns glass on there applies after a restart, and Settings says so.

**Examples.** Three working themes are in [`docs/theme-examples/`](theme-examples/): zip a folder to make a `.lulltheme`.
- **[Glass window](theme-examples/glass-window/)** (the whole window): every tray is glass and the page is translucent paper.

  ```css
  :root[data-window] :is([data-region="rail"], [data-region="header"], [data-region="playerbar"], [data-region="ctx"], [data-region="nowcol"]) {
    background: color-mix(in srgb, var(--panel) 45%, transparent);
  }
  :root[data-window] [data-region="content"] { background: transparent; }
  /* translucent cards over glass */
  :root[data-window] .main > .view { background: color-mix(in srgb, var(--panel) 75%, transparent); }
  ```

- **[Glass rail](theme-examples/glass-rail/)** (only the side menu). The desktop shows only where every layer down to the window is see-through. So the backdrop is clear (`"opacity": 0`), the other parts are solid, and they meet edge to edge, because a gap would show the desktop too:

  ```css
  :root[data-window] .app { padding: 0; gap: 0; }
  :root[data-window] :is([data-region="header"], [data-region="content"], [data-region="playerbar"], [data-region="ctx"], [data-region="nowcol"]) {
    margin: 0; border-radius: 0; box-shadow: none; background: var(--sky);
  }
  :root[data-window] [data-region="rail"] {
    margin: 0; border: 0; border-radius: 0; box-shadow: none;
    background: color-mix(in srgb, var(--panel) 75%, transparent);
  }
  ```

- **[Glass player bar](theme-examples/glass-playerbar/)** (only the player bar) works the same way, with the bar translucent instead.

**Keeping text readable.** You can't know the listener's desktop, so check against the worst ones: pure black and pure white.
- Text needs 4.5:1, and large text 3:1, against what lies under it composited over a black *and* over a white desktop.
- **The preview** shows exactly that: `#/home?window=blur` in a browser (any material) shows your theme as glass over a black and white checkerboard. Text that reads on both squares reads on any desktop.
- **Rough minimums** for the layer under body text, at 4.5:1:
  - dark text on a light tint: about 0.6;
  - light text on a dark tint: about 0.65;
  - a mid-tone tint needs more: lull's night `--sky` under its cream text needs 0.83.
- The default 0.85 keeps every built-in design's text readable.
- Layers add up. A card at 0.75 over a backdrop at 0.6 covers 1 − 0.25 × 0.4 = 0.9 of the desktop.
- `mica` is the gentlest material. `transparent` and `blur` let the desktop through as it is, so check those.

**Staying fast.** The system draws the glass; lull adds nothing that runs while the window is idle. `backdrop-filter` in your CSS blurs the page, not the desktop, and costs work on every frame that scrolls under it: keep it off large areas.

## 13. Sounds and easter eggs: packs

A theme can sound like its world and hide small surprises, and so can a pack on its own that a listener adds to any
look. Both are declarations: no code, checked by the hub as a theme is. The decision behind it is ADR-0081.

### What a pack is

A pack is a `pack.json` with the files it names:

| File | What |
|---|---|
| `pack.json` | The sounds and the easter eggs (below) |
| `sounds/*.ogg` | Ogg Opus or Ogg Vorbis, each named by `pack.json` |
| `pack.css` | Optional: your eggs' looks. The same rules as `theme.css`, loaded after it, in the `user` layer |
| `slots/egg.<name>.html` | Optional: markup an egg shows while it is live, with the fields `{{count}}`, `{{word}}`, `{{title}}`, `{{artist}}`, `{{hour}}` and the `icon` part |

- **In a theme:** put them beside `theme.json`. `pack.json` then has no `id`, `name` or `author` (the theme's are used). Importing the theme brings its sounds and eggs.
- **On its own:** a `.lullpack` file, a zip of them, with `id`, `name`, `author` and an optional `description` in `pack.json`. Settings › Sounds imports it, and so does dropping it on Your themes or Theme Studio.

```json
{
  "id": "rain-on-glass", "name": "Rain on glass", "author": "you", "api": 1,
  "sounds": {
    "play": "sounds/tap.ogg",
    "skip": ["sounds/flick-1.ogg", "sounds/flick-2.ogg"],
    "buffering": { "file": "sounds/drizzle.ogg", "loop": true },
    "seek": null
  },
  "eggs": [
    { "name": "rainy-song", "when": [{ "words": { "in": ["title"], "any": ["rain", "lluvia", "pluie", "雨"] } }],
      "sound": "sounds/thunder-far.ogg", "line": "Rain on the way", "once": "song" }
  ]
}
```

lull's own packs are in `web/packs/` (`default` and one per design), made by `scripts/sounds/` from code. Read
`web/packs/default/pack.json` and `pack.css` to see every egg lull has and how it draws them; a design's pack draws
lull's eggs its own way in its `pack.css`, and adds a few of its own.
A built-in pack's egg slots are static files too: `render.mjs` declares each egg that has a `slots/egg.<name>.html` in the pack, and lists them in `web/packs/index.json` for the page.

### Sounds

| Event | When |
|---|---|
| `boot` | lull starts: once per app start, never on a reload |
| `play`, `pause` | the listener presses play or pause |
| `skip` | the listener moves to the next or the previous song |
| `seek` | the listener lets go of the seek bar (off until the listener turns it on) |
| `buffering` | a loop while lull waits for sound (finding, queued, buffering), only in silence |
| `found` | a song lull was finding has a source |
| `stalled` | a transfer stalled |
| `saved` | a song the listener saved is in the library |
| `liked` | the listener likes a song |
| `queued` | the listener adds to the queue |
| `error` | a song failed, or an action the listener took failed |
| `design` | the listener switches design or theme |
| `classic` | Classic mode opens |

- A sound is a file, a list of files (lull takes them in turn, so a press repeated never sounds mechanical), or
  `{ "file", "loop": true }` for `buffering`. `null` silences an event on purpose; an event you leave out comes from
  the pack under yours.
- **Keep them short:** at most 2 s (`boot` and an egg's sound 6 s, a loop 20 s), 128 KB a file, 1 MB in all.
- **Loudness is lull's:** it levels every sound as it decodes it, to one quiet peak and loudness, and the listener sets
  the volume. Make yours as loud as you like; nobody hears one pack louder than another.
- **What lull guarantees,** whatever a pack declares: no sound but play and pause over music (the listener can allow
  more), the loop never over music and gone the moment the song sounds, nothing while lull isn't in front (but the
  four the listener asks to hear in the background), nothing while the system asks for quiet or reduced motion, and
  nothing while Classic mode is open.

### Which pack a sound comes from

Per device, from the top: a sound the listener replaced; the packs they added, in their order; the look's pack (your
theme's, else the design it extends); then `default`. A sound comes from the first pack that has it. Eggs come from
every pack that is on; two that declare the same `name` are one egg, the higher pack's declaration winning. So a
theme re-declares one of lull's eggs to give it its own sound or look without a second copy of it.

### Easter eggs

An egg is `{ "name", "when": [conditions], ...reaction }`: when all its conditions hold, lull plays its reaction.

| Condition | Parameters | Holds |
|---|---|---|
| `clock` | `from`, `to` (`HH:MM[:SS]`, the device's time; may wrap past midnight; `to` may be `"24:00"`), `date` (`MM-DD`, or a list), `weekday` (`mon`...`sun`, or a list), `countdown` (true: `--egg-count` is the seconds left until `to`) | while the time is inside. A window past midnight belongs to the date it starts on |
| `words` | `in` (`title`, `artist`, `album`; a list), `any` (1-40 words, any language and script) | while the current song's words match: whole words, ignoring case and accents, and inside the text for scripts written without spaces. No plural is assumed: list the forms you want. `data-egg-word` is the word as you wrote it |
| `repeat` | `times` (2-100) | the current song has played through that many times in a row |
| `gesture` | `on` (`brand`, `cover`, `title`), `taps` (2-12), `within` (s, 0.5-5) | at the tap that completes the count |
| `milestone` | `of` (`likes`, `follows`), `at` (a number or a list) | as the count reaches one; `--egg-count` is that number |
| `player` | `status` (a player status or a list), `playing` (true: sound is coming out), `screen` (a `data-screen` or a list), `idle` (s without a tap, key or scroll), `held` (s these have held together) | while they all hold |
| `event` | `on` (a sound event) | at that event |
| `chance` | `one_in` (2-1000) | with that chance, each time the rest hold |

| Reaction | What |
|---|---|
| `name` | Required. While the egg is live, `:root[data-egg~="<name>"]` (several may be live at once), with `--egg-count` and `data-egg-word` on `:root` when its conditions give one. Style it in `pack.css` or `theme.css` |
| `sound` | A file played under the sound rules. With `"instead": true` it takes the place of the sound of the `event` that fired it, or, as `{ "file", "loop": true }` with a `player` condition, of the buffering loop while the egg is live |
| `line` | Words lull shows in `.egg-line` and reads out politely, with `{{count}}` and `{{word}}` |
| `go` | An in-app place to open (`#/credits`, lull's credits) |
| `for` | Seconds it stays live after its conditions stop holding, or after a moment (0.5-600, 4 to start) |
| `once` | `always`, `song` (the default), `session`, `day`, `night` (18:00 to 06:00), `week`, `year`: it fires at most once in each |

A slot `slots/egg.<name>.html` is drawn into `.egg-stage` while the egg is live, in a box `[data-egg-of="<name>"]` you
place with your CSS. `.egg-stage` covers the window, above the page, and takes no pointer.

**Rare and never in the way.** An egg never fires while a dialog or sheet is open, and every live egg ends at the
listener's next tap or key. Hidden surprises in Settings › Sounds turns them all off; behind its spoiler each has its
own switch and a Try it button, and so does Theme Studio's list of your theme's eggs. **Cheap by construction:** lull
checks conditions on what already happens (a song or status change, a like, a tap on the three targets, one timer for
the next clock edge), never by polling. Your looks follow section 5's rules (Drawing cheaply) and stay still under
reduced motion; a loop in an egg's look takes `animation-play-state: var(--loop-play, running)` like any other.

**What the hub refuses**, naming the problem: a file that isn't Ogg (read from its headers), a sound nothing names or
a name with no file, an event, condition, parameter or field lull doesn't know (the message lists those it does), a
value out of its range, a sound too long or too large, more than 32 eggs, an egg slot for an egg the pack doesn't
declare, and `pack.css` or a slot under section 8's rules.

## 14. The live model: lull's values, anywhere

Everything that changes while lull runs, the clock, the song, what plays next, the player's state, reaches a theme
the same way: as one model of named values, listed in `web/designs/live.json` (the decision is ADR-0101). A theme names
the values it wants, where it wants them, and lull keeps them current there. A value lull adds later reaches every
theme the same way, with no new hook.

**A value's path** is dotted, `clock.minute`. Its type gives the forms it takes:

| Type | Words, in a slot | Custom property, on the live hosts | On `:root` |
|---|---|---|---|
| `text` | `<b data-bind="clock.time"></b>` | `--clock-time`, a CSS string (for `content:`) | |
| `number` | `data-bind`, as the value's words (`clock.hour` reads `07`) | `--clock-hour`, a number (for `calc()`, `rotate`, `counter()`) | |
| `state` | `data-bind`, as its words (`clock.weekday` reads `Sunday`) | `--clock-weekday`, the state's name as a CSS string | `data-clock-weekday="sun"` |
| `flag` | | `1` or `0` | `true` or `false` |

- **Words** go in any slot (section 6): `<span data-bind="clock.hour"></span>:<span data-bind="clock.minute"></span>`,
  and are kept current in place, in lull's page and in the desktop app's mini player windows alike. A slot's field takes
  the same path, `{{clock.time}}`, as the slot is drawn.
- **Custom properties** are written on the *live hosts*: `#pbar`, `#mini`, the mini player windows' `[data-dk-player]`,
  Now Playing's hero (`.now__dev`, `.now__devc`), Home's lead (`.disc-lead`), `.sky-specks`, every `[data-region]`, and
  any element of your slot markup you mark `data-live`. Like `--song-pos`, they don't inherit, so a change restyles the
  host alone: a part inside that draws with one says `--clock-minute: inherit` (and each level between). A clock hand:
  ```css
  .my-clock { --clock-minute: inherit; --clock-hour: inherit; }
  .my-clock__min { rotate: calc(var(--clock-minute) * 6deg); }
  .my-clock__hour { rotate: calc(var(--clock-hour) * 30deg + var(--clock-minute) * .5deg); }
  ```
  A number shows as words from CSS alone with a counter: `counter-reset: m var(--clock-minute)` then
  `content: counter(m, decimal-leading-zero)`.
- **States and flags** are attributes on the root of every window, so one rule follows them anywhere:
  `:root[data-clock-daypart="night"] .sky-specks { ... }`.

**The values:**

| Path | Type | What |
|---|---|---|
| `clock.time` | text | the time as the device writes it: `14:12`, or `2:12 PM` |
| `clock.hour`, `clock.minute` | number | 0 to 23, 0 to 59; as words two digits (`07`, `05`) |
| `clock.second` | number | 0 to 59; the only clock value that changes each second, so name it only to show seconds |
| `clock.weekday` | state | `sun` `mon` `tue` `wed` `thu` `fri` `sat`; as words the day's name in the device's language |
| `clock.date` | number | the day of the month |
| `clock.month` | number | 1 to 12; as words the month's name in the device's language |
| `clock.year` | number | `2026` |
| `clock.daypart` | state | `morning` from 5:00, `afternoon` from 12:00, `evening` from 18:00, `night` from 22:00 |
| `song.title`, `song.artist`, `song.album`, `song.year` | text | the playing song; empty while nothing plays |
| `song.from` | text | the place it plays from, as the player bar names it: `Liked songs`, a playlist's name |
| `song.source` | state | where the copy that plays comes from: `none` `library` `device` `soulseek` `youtube` |
| `song.quality`, `song.format` | text | the copy's quality as its chip says it (`FLAC 16/44.1`), and the file's facts (`FLAC :: 44.1 kHz, 1123 kbps, 32.8 MB`) |
| `song.lossless`, `song.liked`, `song.saved` | flag | |
| `song.pos`, `song.buffered`, `song.received` | number | 0 to 1 in hundredths (as words a percentage): where it is, how far it is buffered, how much of the file has arrived |
| `song.elapsed`, `song.duration` | text | `1:23`, `3:45`; `song.elapsed` changes every second, so name it only to show it |
| `song.hue`, `song.seed` | number | for decoration, the same for a song every time: 0 to 359, and 0 to 1 in thousandths |
| `song.title-length` | number | how many characters the title has, to size type by: `font-size: clamp(3rem, calc(90vw / var(--song-title-length)), 12rem)` |
| `next.title`, `next.artist`, `next.album` | text | the song after it; empty at the end of the queue |
| `next.saved` | flag | |
| `queue.1.title` ... `queue.8.title`, `queue.1.artist` ... `queue.8.artist` | text | the eight songs after the playing one |
| `queue.count`, `queue.left` | number, text | how many songs come after it, and for how long (`1 h 12 min`) |
| `player.status` | state | as `:root[data-player]`: `idle` `finding` `queued` `buffering` `streaming` `stalled` `choose` `library` `failed` `jellyfin-error` `ended` |
| `player.status-text` | text | the state's own words, as `--status-text` (section 5, The player's words) |
| `player.playing`, `player.shuffle`, `player.muted`, `player.crossfading`, `player.private` | flag | |
| `player.repeat` | state | `off` `all` `one` |
| `player.volume` | number | 0 to 1 in hundredths |
| `app.screen`, `app.view`, `app.ctx`, `app.size` | state | as `.app[data-screen]`, `[data-view]`, `[data-ctx]`, `[data-size]`, now on `:root` too, for what is drawn outside `.app` (`.sky-specks`) |
| `services.soulseek`, `services.library`, `services.lastfm`, `services.catalog`, `services.cache` | state | `ok` `warn` `down`, as the status tray |
| `week.time`, `week.plays`, `week.artists`, `week.peak` | text, number | the last 7 days, once Stats has read them |
| `region.width`, `region.height` | number | each region's own size in px, written on that region alone when it resizes (`calc(var(--region-height) * 1px)`) |

A value changes at its own pace: the song's values once a song, a press's at the press, the song's place a hundredth at a time (as `--song-pos`), `song.elapsed` once a second; and only the values you name are read at all. In the desktop app's mini player windows every value but the regions' arrives with the player's state from the main window, so they say what the main window says.

**Any fact by path.** Beyond the named values above, any field of what lull holds is a value: a root, then the field
(`song.no`, `copy.peer`, `album.label`, `queue.3.title`). It takes the forms its field's type gives: words with
`data-bind` or a field, a custom property on the live hosts (`--song-no`, a list as how many things it has), and on
`:root` its own words (`:root[data-copy-where="soulseek"]`). Where a named value and a field share a path
(`song.duration`, the named value's `3:45`), the named value is the one. The hub refuses a path no record has, and
says which fields there are. (The amendment of 2026-11-09 to ADR-0101.)

| Root | What | Record |
|---|---|---|
| `song` | the playing song | `track` |
| `album` | the playing song's album | `album` |
| `artist` | the playing song's artist | `artist` |
| `copy` | the copy of it that plays | `copy` |
| `next` | the song after it | `queued` |
| `queue.<n>` | the songs after it, by position: queue.1, queue.2... | `queued` |
| `player` | the player | `player` |
| `place` | the place the song plays from | `place` |
| `radio` | the radio that adds songs, while there is one | `radio` |
| `clock` | the device's time | `clock` |
| `item` | inside a list part's own row (`<lull-row>`): the row's item, any field its record or its read holds | `item` |

| Record | Fields, with their types |
|---|---|
| `track` | `id` text, `title` text, `artistId` text, `artist` text, `albumId` text, `album` text, `year` number, `no` number, `disc` number, `duration` number, `tags` list, `saved` flag, `format` text, `codec` text, `lossless` flag, `khz` number, `bits` number, `kbps` number, `sizeBytes` number |
| `album` | `id` text, `title` text, `artistId` text, `artist` text, `year` number, `type` text, `released` text, `label` text, `tags` list, `saved` text, `trackIds` list, `duration` number, `musicbrainzId` text |
| `artist` | `id` text, `name` text, `tags` list, `listeners` number, `bio` text, `musicbrainzId` text |
| `queued` | a `track`'s fields, and what lull works out for a song: `source` text (where it would play from: `library`, `device`, `soulseek`), `liked` flag, `kind` text; in the queue, `group` text (`yours`: queued by the listener, `place`: from the place it plays from, `radio`) |
| `copy` | `where` text, `name` text, `library` text, `saved` flag, `received` number, `format` text, `peer` text, `sizeBytes` number, `speed` number, `rank` number, `of` number |
| `player` | `status` text, `statusText` text, `playing` flag, `shuffle` flag, `repeat` text, `volume` number, `muted` flag, `crossfading` flag, `private` flag, `duration` number, `durationKnown` flag |
| `place` | `kind` text, `id` text, `name` text, `path` text |
| `radio` | `name` text, `keepsAdding` flag, `steer` number, `error` text, `seed` text |
| `clock` | `now` date |
| `item` | a `queued` song's fields, or an album's, an artist's or a playlist's (`kind` says which), and any other field its read holds (a play's `at`), as text |

Each record's fields are lull's own, as the hub sends them (`track`, `album`, `artist`) or as the page builds them; a
field lull adds reaches you by its name. A field lull doesn't know for a song (no track number) is empty, and a number
0.

**Pipes: a value in your words.** In a binding or a field, `|` hands the value to a pipe, and pipes chain:
`<b data-bind="copy.sizeBytes | size"></b>` reads `32.8 MB`, `{{ clock.now | date:weekday-short | upper }}` reads
`SUN`, `<b data-bind="player.playing | pick:pause,play"></b>` says what the key will do, `{{ clock.weekday | upper }}`
turns the named state `sun` into `SUN`. A pipe's argument follows a colon. CSS reads the value itself, never piped.
Pipes work on the model's values only: `{{ song.title | upper }}`, not a slot's own `{{ title }}`. A clock shown to the
second (`date:second`, `date:time-seconds`) wakes each second, as `clock.second` does; any other, each minute.

| Pipe | What it gives |
|---|---|
| `size` | a byte count as words: 32.8 MB |
| `time` | seconds as a clock: 3:45, 1:02:03 |
| `length` | seconds as a length: 36 min, 1 h 12 min |
| `date` | a time in the device's language, by its argument: weekday, weekday-short, weekday-narrow, month, month-short, day, year, date, date-short, time, time-seconds, hour, minute, second |
| `upper` | in capitals |
| `lower` | in small letters |
| `pad` | a number with leading zeros to its argument's length: pad:2 makes 07 |
| `pct` | 0 to 1 as a percentage: 42% |
| `fixed` | a number with its argument's decimals: fixed:1 |
| `pick` | the first word for a true value (not 0, empty or false), else the second: pick:pause,play |
| `map` | a value's own words by key: map:sun=SUNDAY,sat=SATURDAY; any other value stays as it is |
| `or` | its argument when the value is empty, 0 or false: or:- |
| `count` | how many things a list has |
| `join` | a list's things joined by its argument (', ' if none) |

**You pay for what you name.** The hub reads which values your CSS and slots name (a `data-bind` or `{{field}}` in a
slot, a `var(--clock-minute)` your CSS reads, a `[data-clock-daypart]` it tests) and lists them in your theme's
manifest, as `live`; lull loads the model's script (`designs/live.js`) only for a theme that names one, and computes
and writes only those. The clock is one timer to the next minute's edge (to the next second only when you name
`clock.second`), and stops while lull is hidden. A custom property your theme sets itself (`--clock-hour: 3`) is your
own: lull leaves it alone.

**What the hub refuses:** a slot that binds or fields a path in one of the model's branches or roots that the model
doesn't have (`data-bind="clock.hours"`, `copy.peers`), naming the ones it has; a pipe lull doesn't have, naming its
pipes; and a pipe on a name outside the model. Names outside the model are your own, as always.

## 15. Building from a mockup

A mockup is your theme's idea drawn whole: an LED over the page that names the song, a pocket player's LCD in the
bar, a column of lyrics and what plays next beside the page. Build it part by part, and make each part one of two
things (ADR-0101):

- **Your own markup**, for what lull doesn't draw: a slot at the edge of the region or screen it sits in (section 6),
  `slots/before.header.html` for the LED, `slots/after.playerbar.html` for the LCD. Its words and states are the live
  model's values (`<b data-bind="song.title"></b>`, `--song-pos`, `:root[data-player-repeat="one"]`, section 14), and its
  controls are plain buttons that name an action (below), with a `data-bind` where lull keeps one pressed
  (`<button data-act="shuffle" data-bind="shuffle">SHUF</button>`). lull never redraws your markup; it changes the values
  in it, in place.
- **One of lull's parts**, where the mockup shows one: lull's own, styled and placed by the grid of the screen or region
  it is in, or a copy from the catalogue in your markup (`<lull-part name="up-next" count="4">`, section 6). The column of
  lyrics and up next is your markup in `slots/after.ctx.html` (or `before.content`) holding the `lyrics` and `up-next`
  parts, each in a box of yours, laid out by your grid, at the size its content takes. Nothing is fixed in place at
  measured heights: where a size matters, each region's own is in the model (`--region-height`, section 14).

Where you draw your own version of something lull shows, hide lull's on that screen (its card, its tag) rather than
moving it, and keep what it told a screen reader: lull's cards are named (`aria-label`, a heading), so give your box
the same name, or a heading of yours hidden only from sight (`clip-path: inset(50%)` on a 1 px box), never
`display: none` on the words a screen reader needs. The gates still check that the essentials stay visible, named and
clickable whatever you hide (section 5).

**Your design is the design.** lull's checks judge whether a theme works: every control clickable and reachable,
nothing cut off or on top of something else, fast. They never judge its look. themes:check's contrast reading is
information (section 10): match your mockup, and weigh the reading yourself. No haze, shadow or tint the design doesn't
have is needed to pass anything.

**Which screens are already one grid.** Place lull's parts and your boxes as items of one grid rather than stacking
boxes over each other:
- **Now Playing:** `.now__grid` is a grid whose items are the hero (`.now__dev`), the title and actions (`.now-head`,
  `.now__acts`, or `.now__intro` as one box), the source and lyrics cards (`.now-srccard`, `.now-lycard`, or
  `.now__side`) and up next (`.now-next`), placed by `grid-template-areas`; your `before.now` and `after.now` boxes are
  items of it too.
- **Home:** its modules each have a box that names them, `.disc-mod[data-module]` (`picks`, `new`, `radio`, `jump`,
  `tags`), inside `.disc-modules` (columns), after the greeting (`.disc-hello`) and the lead (`.disc-lead`), in
  `.disc-home`, where your `before.home` and `after.home` boxes are too. To place them all in one grid, make `.disc-home`
  the grid and `.disc-modules` `display: contents`: a window behind the greeting and the first module is then a box of
  yours in the same grid area, not a pseudo-element stretched behind two of lull's.
- **The shell:** each region is a child of `.app`, a grid; a region's slot boxes are items of the region's own layout.

**The actions a button of yours can name** (`data-act`), each what lull's own control with that action does. Buttons
for the playing song need no `data-id`.

| `data-act` | What it does |
|---|---|
| `play` | play, or pause |
| `prev`, `next` | the song before, the next song |
| `shuffle` | shuffle on or off (`data-bind="shuffle"` keeps it pressed) |
| `repeat` | repeat off, all, one, in turn (`data-bind="repeat"`) |
| `mute` | mute or unmute (`data-bind="mute"`) |
| `now-vol` | the volume a step up (`data-dir="1"`) or down (`data-dir="-1"`) |
| `like` | like the playing song, or unlike it (`data-bind="like"`) |
| `save` | save the playing song to the library, or try a failed save again (`data-bind="save"`) |
| `add-to-playlist` | add the playing song to a playlist (a picker opens) |
| `start-radio-current` | a radio from the playing song |
| `go-playing` | open the place the song plays from, at the song |
| `source-details` | where the playing copy comes from, and its file |
| `pick-source` | choose another copy of the song |
| `retry-search` | look for the song on Soulseek again |
| `player-more` | the player's "…" menu |
| `connect` | play on another device |
| `toggle-panel` | open or close the side panel at a tab: `data-tab="queue"`, `"lyrics"` or `"activity"` |
| `now-column` | show or hide the Now Playing column |
| `open-now` | Now Playing (in the desktop app, the mini player) |
| `classic` | Classic mode |
| `theme` | day or night |
| `status` | the services' status, by the button |
| `shortcuts`, `palette` | the keyboard shortcuts, the command palette |

**One vocabulary, every window.** These actions work wherever your markup is: in any slot of lull's page, in a list
part's row of yours (with the row's item: `data-act="play-track" data-id="{{item.id}}"`, section 6), and in the desktop
app's mini player windows, where a button of yours with `data-act` runs in the main window as it would there (with
its `data-*` attributes). The mini player's own keys (`data-dk`, section 6) stay as they are.

Screens are links: `<a href="#/library">`, `#/search`, `#/now`, `#/settings/appearance`.
