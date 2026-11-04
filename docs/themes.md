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
- `api`: the theme API version you wrote for (section 9). Today it is `1`.
- `extends`: a built-in design (`lull`, `skydeck`, `starlight`, `petal`, `glasshouse`) whose CSS and slots load first, or `null` to start from the base look alone.
- `shell`: the layout on desktop and tablet: `rail` (a side menu), `topnav` (pills in the header) or `dock`. Phones always use the tab bar.
- `swatches`: page, surface, accent and ink colours (`#rrggbb`) for the picker card in Settings › Appearance.
- `modes`: `["day", "night"]`, or just one. A one-mode theme ignores the day/night switch.
- Optional: `description`, `preview` (true when `assets/preview-day.webp` and `-night.webp` exist, 320×200), `coverVariety` (section 5), `window` (a see-through window in the desktop app, section 12). The hub fills in `slots` itself.

**Installing one:** drop the `.lulltheme` on Settings › Appearance › Your themes, or make it in Theme Studio there. Both store it on your hub in `<data dir>/themes/<id>/`, so every device can pick it. You can also edit the files in that folder directly: open devices reload the theme within a second.

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
| Sizes | `--rail`, `--rail-icons`, `--ctx`, `--bar-h`, `--mini-h`, `--tab-h`, `--hit`, `--tag-h`, `--tag-ic`, `--tag-gap` (every tag's height, icon size, and gap between icon and words) |
| Window (section 12) | `--window-tint`, `--window-opacity` (from theme.json's `window`), `--window-backdrop` (what lies over the desktop where the wallpaper was) |

Components read semantic tokens (`--bg`, `--fg`, `--fg-2`, `--link`, `--edge`, `--focus`...) that the surfaces `.s-sky`, `.s-panel`, `.s-card` and `.s-screen` map from the ones above, so a surface's text always pairs with its own background.

## 5. Stable hooks for full CSS

Go beyond tokens with any selector. These hooks are the theme API: they stay put across lull versions (section 9).

| Hook | Values |
|---|---|
| `:root[data-theme]` | `day`, `night` (always explicit; "match system" is resolved for you) |
| `:root[data-shell]` | `rail`, `topnav`, `dock` |
| `:root[data-motion]` | `reduce`, `full` (absent: follow the system). Keep motion off under `reduce`. |
| `:root[data-design]` | the built-in design that is loaded (your `extends`), else your id |
| `:root[data-window]` | `mica`, `acrylic`, `blur`, `transparent`: what the desktop app's window shows behind the page now (section 12); absent while the window is solid |
| `[data-region]` | `rail`, `header`, `topnav`, `content`, `ctx` (queue, lyrics, activity), `playerbar`, `mini` (phone player), `tabbar`, `lead` (Home's homeLead slot) |
| `.app[data-screen]` | `home`, `search`, `now`, `library`, `liked`, `history`, `following`, `album`, `artist`, `tag`, `playlists`, `playlist`, `stats`, `settings`, `classic`... |
| `.app[data-route]`, `.app[data-view]`, `.app[data-ctx]` | nav key; `page` or `now`; `docked`, `open`, `closed` |
| `[data-act]`, `[data-nav]` | every control's action (`play`, `next`, `like`, `shuffle`...) and nav target (`home`, `library`...) |
| `#pbar` / `#mini` `[data-status]`, `[data-playing]` | player state (`idle`, `streaming`, `buffering`, `failed`...) |
| Classes | the BEM blocks: `.card` (`.card-head`, `.card-head__title`), `.btn` (`--primary`, `--quiet`, `--sm`), `.btn-icon`, `.btn-gloss`, `.cover` (`.c1`-`.c8` recipes, `.has-art`; `.cover--artist` with `.cover__mono`, the artist's initial shown while it has no photo, in the design's display face), `.trow`, `.tile`, `.chip`, `.pill-filter`, `.seg`, `.input`, `.seek`, `.wave`, `.device`, `.empty`, `.sheet`, `.dialog`, `.toast`, `.banner`, `.nav-item`, `.nav-pill`, `.tab`, `.region-title` (hidden window titles you can show). Every other class and attribute, stable or internal, is in section 11 |

**Wide screens (ADR-0007).** A bigger window reveals more (more columns, more tiles); nothing is stretched. Your theme can add its own touches per size:

| Hook | Values |
|---|---|
| `.app[data-size]` | the main area's width class: `compact` (under 640 px), `regular` (to 1099), `wide` (to 1599), `xwide` (to 2199), `ultra` |
| `.app[data-height]` | `short` (under 760 px) or `tall` |
| Containers | `main` (`.main`), `panel` (`.ctx`), `nowcol` (the Now Playing column), `section` (a Home module), `tlist` (a song list), `libpl` (the playlist table): `@container main (min-width: 1600px) { ... }` |
| `.app[data-panel="strip"]`, `.ctx__strip` | the side panel collapsed to an icon strip |
| `.nowcol`, `.nowcol__head`, `.nowcol__title`, `.nowcol__body`, `.app[data-nowcol="open\|strip"]` | the Now Playing column beside the main area |
| `.disc-modules > .disc-mod > .card[data-region]` | Home's modules (`new`, `radio`, `picks`, `jump`, `tags`), in a fixed order |

**Never patch a width band.** The header, the player bar, its tools and the song's chips fit the room they actually
have (a docked panel, the Now column, the text size and a long title count as much as the window): each part that can
give way has an order (`data-fit`), and when a row runs out of room its parts give way in that order and come back with
room. So a theme never writes `@media (min-width: 1100px) and (max-width: 1359px)` to hide a tool or shrink a pill: style
`[data-fit-out]` for how a part looks once it gave way, keep the bar's side columns equal (`minmax(0, 1fr)`) and let a
part's track be `auto`, so a part that gives way takes its room with it. Width rules are for genuine layout changes: the
shell breakpoints (640, 1100 and 1360 px) and content that rearranges by its container's room (`@container`).
`mise run check:ui` sweeps every width from 360 to 2560 px for the built-in designs (docs/context/quality.md#ui-gates).

Prefer `@container main` to `@media`: it follows the main area, so it stays right when a panel opens. Layout states that must hold whatever a theme draws (the strips, the room the column takes) are `!important` in lull's own CSS: style them, don't fight them. The widths the listener drags are custom properties (`--ctx`, `--now-w`); read them, don't set them. While a drag runs the column's width is `--ctx-drag` / `--now-drag` instead, so a theme's own shell grid that places the panel writes its track as `var(--ctx-drag, var(--ctx))`.

`design/DESIGN.md` section 3.4 lists more state hooks (tray pills, track row file facts). Pseudo-elements are free for ornaments; keep them decorative.

**Measured parts.** lull places a `.seg`'s indicator (`.seg__ind`) on the selected button by measuring it (`--seg-x`, `--seg-w`), so gaps, padding and labels of different lengths are fine: restyle or hide the indicator, but don't position it. A `.wave`'s bars share its whole width (set the `gap`, not the bars' width), and its head travels the strip less its own width, so it never passes the last bar: a bigger head sets `--wave-head` on the `.wave` and fills `.wave__rail`.

**Staying fast.** Chrome re-checks a `:has()` on a page-wide element (`:root`, `.app`, `.main`) whenever anything is inserted anywhere: a menu, a toast, a tooltip, a row. Each time it restyles every element the rest of the selector could match, so `:root:has(X) .now-card :is([data-act="retry"])` or `... span:not(:last-child)` restyles the whole page on every such change. End those selectors in a class, prefer a state lull already sets (`.app[data-screen="now"]` rather than `.app:has(.now)`; `.pipeline-wrap--idle` for a save pipeline that hasn't started; `.wave-times__buf` for the "buffered to" part of the times), and never test classes by substring (`[class*="is-"]`) inside `:has()`. Animate `transform` and `opacity` only, so loops run off the main thread, and draw them cheaply (below). `mise run check:ui` measures all of this for the built-in designs (docs/context/quality.md#ui-gates).

**Drawing cheaply.** A loop that never ends (a twinkle, a blinking caret, a ticker, the playing bars) keeps the GPU drawing while the listener is away, as often as the screen refreshes (165 times a second on a fast monitor) unless it says otherwise. Keep your loops, and make each one cheap:
- **Name its rate.** Set `--loop-fps` on what the loop animates (the element or pseudo-element with the `animation`). lull steps the loop to that many changes a second and keeps its easing (the steps sample it). It puts every such loop on one beat, so the compositor draws a frame only when a loop has changed, and loops at 10 and 20 a second (or 10 and 30) together draw no more frames than the faster one. Pick the slowest rate that looks the same: 10 for a slow twinkle or fade (lull's sparkles, Skydeck's sky), 20 for a quicker twinkle or a ticker (about a pixel a step), 30 for what moves with the music (the playing bars). A `steps()` loop (a blinking caret) names 10 to land on the beat. A duration in whole tenths of a second keeps a loop on it.
- **Damage only what moves.** Animate `transform` and `opacity` only, on the smallest box that moves. A glow is its own pseudo-element that fades and grows, never an animated `box-shadow`, `filter`, `background-position` or `clip-path`: each of those repaints on the main thread at every step. Skydeck's down LED is the pattern: the pill draws the glow on `::before`, anchored to the LED, and breathes it by transform and opacity.
- **Keep loops off glass.** A `backdrop-filter` blurs again whatever changes under it or inside it, every frame: a twinkle under Glasshouse's frosted queue kept the GPU fully busy. No loop sits under, or inside, an element with a live `backdrop-filter`.
- **Bake what's static.** Glass that stays put over your own wallpaper (the shell's panels) needs no live blur. Draw the wallpaper's smooth layers again under the glass's tint, with `background-attachment: fixed` so they line up with the wallpaper behind, and leave out the fine detail (stars, thin rings) that a wide blur wipes out anyway (Glasshouse's `--gh-wall`). Gradients, shadows and noise that don't move are drawn once and kept; they cost again only when something animates them or something under a filter changes. Glass that moves (a window on the scrolling page) keeps its live blur, since a fixed background under it repaints it on every scroll frame, unless a loop lives in it (Glasshouse's AIMP window and its ticker): then bake it, as one repaint while scrolling costs less than a blur redone at every step. Keep a live `backdrop-filter` wherever the page's own content passes under it: a sheet, a menu, a toast, a panel that slides over the page.
- **Hold still when asked.** A loop that names a rate holds still while `:root.is-still` (lull's one rule for every reason to: so far the listener's "Pause animations when lull isn't the active window", Settings › Appearance, off by default). lull holds it by its playback rate, so your own `animation-play-state` rules (a ticker paused on hover) keep working, and your entrances still finish. A loop that shows something happening (a spinner, a loading sheen, a transfer) sets `--loop-holds: no` and keeps going. Style `:root.is-still` yourself only to still more.
- **Keep big decoration in the page's own layer.** Put your wallpaper and its specks on `.sky-specks`, which lull places absolutely under the app (the page never scrolls, so it covers the window) and clips at the window's edge. A `position: fixed` layer as large as the window becomes a compositor layer of its own, and the app painted over it needs another as large: each is a window-sized texture in GPU memory, drawn again every frame. Animate only what has to move; an animated layer as large as the window (Skydeck's twinkling sky) costs a window too, so keep it to one.
- **Theme Studio checks it with you.** It names each loop that never ends and costs more than it needs to, says what that costs and the cheaper way, and so does Settings › Appearance when you import a theme. It's a hint: the theme works either way.
  - **No rate:** with the rate this guide suggests (20 when it can't tell what the loop shows, and it says so), and an Add it that writes `--loop-fps` after the loop's `animation`. A rate you add or change takes hold at once, in Studio's preview and on open devices when the theme reloads.
  - **Redrawn at every step:** a loop whose keyframes animate anything besides `transform` and `opacity` (`translate`, `scale` and `rotate` count as transform), with the cheaper way for what it changes: a glow on a `::before` that fades and grows, a pattern moved by `transform`, motion by `translate()`, growth by `scale()`, a colour faded in on a layer. There's no Add it: no rewrite of these is sure to look the same.
  - **Under glass:** a loop inside an element with a live `backdrop-filter` (its own rule blurs, or the selector puts it inside one that does, in your theme or the design you extend, as the cascade leaves it), and, in Studio, a loop its day and night previews show passing under glass on the screen they show (switch the screen to check others). From the CSS alone nobody can tell what passes under what, so an imported theme with glass is told that, and pointed to Studio. The advice is the two above: move the loop out, or bake the glass.
  - **What it reads:** the rule the loop is in, its `@keyframes`, a rate set on the same selector elsewhere in your theme, in lull's own CSS or in the design you extend, and `:is()` and `:where()` lists written out. A loop that an `@supports` block switches off (`animation: none`) on the same element is a fallback for browsers without what its better version needs, and is left alone (Skydeck's down LED).
- **The gates** check all of this. gates/perf.spec.mjs checks at rest, with a song paused in day and night, and while playing on Now Playing: every loop on screen names a rate, moves only by transform and opacity, and stays clear of live glass; a page at rest draws at most 22 frames a second; and the compositor keeps no more than the design's budget of layers, counted in windows of area (each layer up to one window). gates/loops.spec.mjs checks, in Chromium and in WebKit (Linux's app, Safari), that every loop is stepped to its rate and holds still when asked. A rate is changes per second of motion, so a loop draws the same frames on a 60 Hz screen as on a 165 Hz one.

**Cover variety:** with `"coverVariety": true` every cover gets stable per-item hooks, so two albums that share a generated cover still differ: `data-look="0"`-`"5"` (pick one of 2, 3 or 6 looks with `[data-look="1"], [data-look="3"], [data-look="5"]`, ...) and `--cover-flip` (1 or -1), `--cover-hue` (-28deg to 28deg), `--cover-zoom` (1 to 1.45), `--cover-dx`, `--cover-dy`.

**Parts with states of their own.** These hold rules a theme must keep, so style them, don't rebuild them:

- **The song row's number cell** (`.trow__num`) shows one thing at a time: the number (`.trow__n`), Play (`.trow__play`) under the pointer or keyboard focus, or the playing bars (`.eq`, drawn in `.trow.is-playing` only). Under 520 px the cell is gone and the cover is the playing marker (`.trow.is-playing .trow__art::after`).
- **The playing bars** (`.eq`, three `<i>`) move only inside `.is-playing-now` (sound is coming out), and stand still while paused. Animate them with `transform` only.
- **The place playing** (a tile, a rail playlist, a search result, a menu item: `[data-place].is-playing-place`) gets `.place-mark` (the bars, plus a hidden "(playing)") before its title. "Playing from" is `.from` (icon, `.from__label`, the place's name in `b`): `.pbar__from` in the player bar, `.mini__from` in the phone mini player, `.now-head__from` on Now Playing. Clicking it opens the place, where the playing row is lit for 1.8 s with `.is-revealed`.
- **Covers.** `.cover` holds a recipe (`.c1`-`.c8`, drawn on `::before`/`::after`) until its picture loads; then `.has-art` hides the recipe. A cover still waiting for the hub keeps its recipe, so a recipe must look finished on its own. `.mosaic` is a playlist's 2×2 of covers, and `.mosaic--one` holds one picture: the playlist's own, or the one album its songs share.
- **The cover viewer.** `.cover-zoom` wraps the player bar's and the Now column's cover, and its `.cover-zoom__btn` shows on hover and keyboard focus. The button is never shown on touch screens. The viewer is a `.dialog.cover-view` holding `.cover-view__art` and `.cover-view__info` (`__title`, `__by`). Its picture is as large as the window allows, up to 1024 px: give it a frame, not a size.
- **Sortable column headers** on the playlist page (`.lib-thead > .lib-th`) carry `aria-sort="ascending|descending"` on the sorted column. The arrow is `.lib-th__btn .ic`, and `.lib-th--added` is the date column. While the list is sorted, `.lib-sortnote` says why dragging is off.
- **Tags** (`.badge`, `.q-chip`, `.badge-new`, `.device__fact`, `.pbar__private`, `.mini__private`) are one box: `--tag-h` tall, border-box, contents centred, an icon `--tag-ic` square and `--tag-gap` from its words, so tags side by side are one height on one centre line with one icon size. Size them with these tokens, on `:root` or on the row that holds them (`.tile__meta`, `.pbar__badges`), never with a tag's own `height`, `gap` or icon size. A wrapper that holds only a tag (`.src-badge`) is only that tag's box.
- **The player bar** keeps the transport centred on the bar: its three columns (`.pbar__now`, `.pbar__centre`, `.pbar__tools`) keep equal sides in every design, so change their gaps, not that balance. Every control has a fixed slot, and toggles show pressed or not (`aria-pressed`). Only width breakpoints move tools into the "…" menu (`.pbar__tools .is-wide` are the ones that go). The status line is `.pbar__state`, a tab on the bar's top edge that is `[hidden]` while all is well, with `.is-error` for a failure. `.pbar__xfade` floats above the tools during a crossfade, and `.pbar__private` is the "Private" tag. The two times are as wide as the song's duration reads (`--time-ch`), so the seek bar keeps its length.
- **Sliders** (`.seek`, `.range`, `.vol`): `.seek.is-scrubbing` while the pointer drags it. A slider clicked rather than tabbed to has `[data-pointer]` and no focus ring; the ring comes back with the first key. Show focus with `:focus-visible`, and never on `[data-pointer]`.
- **Segmented controls** (`.seg`): place `.seg__ind` by the measured `--seg-x` and `--seg-w` (above).
- **The side panel's tabs** (`.ctx__head .seg__btn[data-ctx-tab="queue|lyrics|activity"]`) and the phone Now Playing tabs (`.now-seg`) lose Lyrics for a song with no words (none found, or instrumental), and get it back when a song has some. Never give a tab a fixed position by count.
- **Classic's section in Settings** (`/settings/appearance`): `.cls-now` (the skin in use), `.cls-browser` (tabs and search over `.cls-grid`), `.cls-card` (`.is-applied` for the skin in use, `.cls-card__status.is-bad` for one that failed) with its `.cls-shot`, the larger view `.cls-pop` > `.cls-look`, `.cls-nsfw`, and Your skins' `.cls-folder` (where the hub's folder is, with Look again) and `.cls-skipped` (the files it skipped). Keep `.cls-shot`'s 275:348 ratio and its sharp pixels. The stage itself is Webamp alone on `.cls-desk` / `.cls-scrim`; the app under it is hidden while `:root[data-classic="on"]`.
- **The desktop app's own pages** load your theme too: the mini player windows (`.dk-mini`, and `.dk-strip` in `.dk-strip-wrap`, with `slots/miniPlayer`), and the connect, unreachable and close-to-tray pages (`.dk-page`, `.dk-card`, `.dk-form`, `.dk-hubs`, `.dk-modal`). They come from the app, not your hub, so the app's window hands them your theme once it has loaded it: `theme.css` with every `url("assets/…")` inlined, and your slots. Your theme shows in the mini player, and on the unreachable page when the hub goes down while lull runs. An edit on the hub reaches an open mini player. Until a page has your theme, it shows the design you `extends` (lull when you extend none): after the app starts with the hub down, or for the moment before the window has loaded it. Reach your files only through `url()` in `theme.css`: that is what gets inlined. The title bar is `.dk-titlebar` with `.dk-caption` buttons, under `:root[data-titlebar="win|gnome"]` and `:root[data-window-inactive]`. The mini windows are transparent (`:root.dk-transparent`), so the player's own shape is the window: never paint the page `body` there. The Classic window (`desktop/winamp.html`) is Webamp on nothing and takes no theme.

## 6. Slots: your own markup

Six pieces of markup can be replaced by a template in `slots/`:

| Slot | Where | Variants (`slots/<slot>.<variant>.html`) |
|---|---|---|
| `brand` | the wordmark in the rail, header and desktop pages | |
| `emptyArt` | the picture of every empty or error state | |
| `homeLead` | Home, between the greeting and the shelves | `playing`, `idle` |
| `nowHero` | the Now Playing hero | `full`, `compact` (phone tabs, short screens) |
| `miniPlayer` | the desktop app's mini player windows | `device`, `strip` |
| `coverArt` | every cover | no template: use `coverVariety` |

lull looks for `slots/<slot>.<variant>.html`, then `slots/<slot>.html`, then your `extends` design's, then its own. An empty file renders nothing (an empty `homeLead.idle.html` hides the lead while nothing plays).

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

Keep the live hooks when you rebuild a hero: `data-now="title|artist|album|format|screen|c-screen|c-art|led|head|state|facts|osd"` and `data-act` / `data-bind` on transport buttons (`design/DESIGN.md` 3.5 lists them all). A custom mini player keeps `data-dk-player`, the `data-dk="..."` keys and `data-dk-pos`, `data-dk-dur`, `data-dk-now`. The keys are `play-pause`, `previous`, `next`, `like`, `shuffle`, `repeat`, `vol-up`, `vol-down`, `seek` (on the wave), `full` and `mini-close`, the names `web/desktop/mini-keys.json` lists. The hub refuses a slot that names any other key, and says which keys there are, so a theme never ships a button that does nothing.

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
- an SVG with a script or outside reference, or any file that isn't `theme.json`, `theme.css`, `slots/*.html` or a font, image or `.txt` in `assets/`;
- a `theme.json` field lull doesn't know, a `window` material it doesn't know (section 12), or an `api` newer than this lull.

In the browser every template is also sanitized (DOMPurify), and the page's Content-Security-Policy only loads fonts, images and styles from the hub itself (`'self'`, `data:`, `blob:`).

## 9. Versioning

- `api` is the theme API version a theme is written for. lull speaks one version (now `1`) and refuses a theme that asks for a newer one, so update lull to use it.
- The API is: the tokens in section 4, the hooks in sections 5 and 11 (the stable ones), the slots, fields and parts in section 6, and the package format. Adding a token, hook, field, part or slot keeps the version. Section 11's internal hooks may change in any release.
- Renaming or removing any of them, or changing what one means, is a breaking change: it raises `api`, and the release notes say what moved. A theme written for an older `api` keeps loading; whatever it relied on that moved is ignored, never an error.
- Built-in designs' own classes (`.lull-*`, `.sd-*`, `.sl-*`, `.pt-*`, `.gh-*`) are not part of the API; extend a design for its look, but target the hooks above in your own rules.

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
- tablet 768 and 1024;
- a short laptop, 1366×768;
- desktop 1440×900;
- `wide` 1920×1080;
- `ultra` 2560×1440.

Also a window narrowed past each `data-size` step. Nothing may scroll sideways, and no title may be squeezed.

**Every mode** the manifest lists, with each token you set given in both the day and the night block.

**Contrast:**
- **The ratios:** text 4.5:1 and large text 3:1; outlines of controls and the parts that show state (a pressed toggle, a knob, a focus ring) 3:1 against the surface under them. Text on a gradient or glass is checked against its darkest and its lightest stop.
- **In every state:** check them in each of the states above, hover included, in both modes. A hover that only changes colour must still pass.
- **Theme Studio** warns for its token pairs; anything else you draw is yours to check.
- **Never colour alone:** where a song comes from (saved or Soulseek) and every service state keep their shape and icon, never colour alone.

**Motion:**
- **Cheap to run:** animate `transform` and `opacity` only, so it runs off the main thread on a slow phone.
- **Cheap to draw:** each loop names its rate (`--loop-fps`), moves only what moves, and stays off live glass; glass over your own wallpaper is baked (section 5, Drawing cheaply).
- **Calm by default:** one signature moment at most. Loops only while they mean something, such as the playing bars while sound comes out.
- **Off under reduced motion:** nothing moves under `:root[data-motion="reduce"]`, or under the system's reduced-motion setting unless `data-motion="full"`. lull's base CSS already stops animations and travel there; don't undo it with `!important`.
- **Paused when hidden:** loops pause on `:root.is-hidden` (the page is hidden).
- **Still when asked:** decorative loops hold on `:root.is-still` by themselves when they name a rate; give a progress loop `--loop-holds: no` (section 5, Drawing cheaply).
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

**Run the checks:**
- **`mise run check`** includes this document's own gate (`tests/web/theme-hooks.test.mjs`): every hook lull's CSS and markup use must be in section 11, and a built-in design may not style a hook that no longer exists.
- **`mise run check:ui`** runs the UI gates on every built-in design: performance budgets, the key screens and accessibility (docs/context/quality.md#ui-gates).
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
  - `data-window` (`mica` `acrylic` `blur` `transparent`: the desktop app's window shows the desktop behind the page, section 12; absent while it is solid) and `data-window-preview` (the browser's preview of it, over a black and white checkerboard);
  - `.is-hidden` on `:root` (the page is hidden: loops pause);
  - `.is-still` on `:root` (decorative loops hold still: the listener's "Pause animations when lull isn't the active window", section 5, Drawing cheaply);
  - `--loop-fps` on what a loop animates (the changes a second it needs), and `--loop-holds: no` on one that shows progress (section 5, Drawing cheaply).
- **The app:**
  - `.app` with `data-screen`, `data-route`, `data-view`, `data-ctx`, `data-size`, `data-height`;
  - `data-panel` (`strip`), `data-panel-split` and `data-two` (Queue and Lyrics as two panes), `data-nowcol` (`open` `strip`), `data-nowcol-ok`;
  - `.is-resizing` (a resize handle is being dragged).
- **Regions:** `data-region` (section 5), `.region-title`, `.region-meta`.
- **Shell parts:** `.rail`, `.header`, `.main`, `.view`, `.ctx` (with `data-ctx-tab` on its tab buttons), `.ctx-scrim`, `.nowcol`, `.pbar`, `.mini`, `.tabbar`, `.tab`, `.nav-item`, `.nav-pill`, `.wordmark`, `.brand` (the default brand slot), `.skip-link`, `.sky-specks`, `.col-resize` (a resize handle, `data-resize`).
- **Links:** `data-nav` on every nav link, `data-brand` on brand links.
- **Rows that fit themselves** (section 5, Wide screens): `data-fit-row` on a row (the header, the player bar, its tools,
  the song's chips), `data-fit` on a part that can give way (its value is its order, 1 first), `data-fit-out` on a part
  that gave way. Style how a part that gave way looks (a nav pill as its icon); lull's own CSS already takes away what
  must go, and puts it in the "…" menu.
- **Status tray:** `.tray` with `.tray__pill[data-svc][data-state]`, `data-worst`, `data-cache-used`, `data-cache-cap`, `data-tray`; `data-activity-count` holds the number of running jobs.
- **Banners:** `.banner`, `.banner-slot`, `.banner-more`, `.is-lead` (the banner a phone shows when the others collapse).
- **Surfaces:** `.s-sky`, `.s-panel`, `.s-card`, `.s-screen`.

### Components

- **Buttons:** `.btn`, `.btn-icon`, `.btn-gloss`, `.btn-save` (`data-state`), `.kbd`, `.switch`, `.switch-row`.
- **Every control:** `data-act` names its action, `data-tip` its tooltip, `data-bind` the player state it shows.
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
- **Source picker:** `.src-table`, `.src-row`, `.score`.
- **Inputs:** `.input`, `.field`, `.search-field`, `.seg`.
- **Overlays:** `.sheet`, `.sheet-bar`, `.dialog`, `.scrim`, `.popover`, `.menu`, `.menu-list`, `.tooltip`, `.toast`, `.toast-region`, `.shelf-pop` (the rail's tags and playlists on a tablet), `.shortcut-list`, `.shortcut-row`.
- **Status and empty states:** `.status-list`, `.status-row`, `.empty` (with `data-art` and `data-title` in its art; `data-noun` in slot templates), `.skel`, `.skel-row`, `.skel-tile`, `.skel-group`, `.sparkle`.
- **Type and icons:** `.t-display`, `.t-title-xl`, `.t-h2`, `.t-row`, `.t-body`, `.t-meta`, `.t-micro`, `.t-px`, `.t-lyric`, `.px`, `.link`, `.link-plain`, `.ic`, `.ic-16`, `.ic-24`.

### States

- **Playback:**
  - `.is-playing` (the row or place playing), `.is-playing-now` (and sound is coming out), `.is-playing-place`, `.is-revealed`;
  - `.is-current`, `.is-past` (a lyric line; `.is-current` also marks a pipeline step);
  - `.is-buffering`, `.is-transferring`, `.is-played`, `.is-buf`, `.is-xfer` (seek and wave parts);
  - `.is-scrubbing`, `.is-shake` (a seek past what has arrived);
  - `.is-power-on` (a device's power-on moment as the song starts);
  - `.is-peak` (the busiest hour in the `week-clock` part);
  - `.sky-star` and `.sky-line` (the `queue-sky` part).
- **Status:**
  - `.is-done`, `.is-failed`, `.is-skipped`, `.is-error`, `.is-below` (below the minimum bitrate), `.is-preferred`, `.is-unavailable`;
  - `.is-liked`, `.is-selected`, `.is-on`, `.is-open`, `.is-wide` (the player bar's view tools: Lyrics, Classic, Device view), `.is-dragging`, `.is-drop-target`;
  - `.is-applied`, `.is-bad` (Classic skins);
  - `.is-out` (Now Playing's slide while the song changes).
- **Player status:** `data-status` (`#pbar`, `#mini`, an album's save status), `data-playing`, `data-state`.

### Home and Search

- **Home:**
  - `.disc-home`, `.disc-hello` (the greeting band), `.disc-lead` (the `homeLead` slot);
  - modules: `.disc-modules`, `.disc-mod` (`data-module`), `.disc-card`;
  - their contents: `.disc-picks`, `.disc-shelf`, `.disc-rel`, `.disc-starters`, `.disc-starter`, `.disc-jump`, `.disc-tags`, `.disc-tag`;
  - loading and empty: `.disc-skel-shelf`, `.disc-picks-skel`, `.disc-cold`, `.disc-empty`.
- **Search:**
  - the page: `.disc-search`, `.disc-q` (the search field on phones), `.disc-tabs`, `.disc-filters`, `.disc-recents`, `.disc-recent`;
  - results: `.disc-duo` (catalog and Soulseek), `.disc-splitview` (side by side) and its switch `.disc-split`, `.disc-top` (the top result), `.disc-songs`, `.disc-scan` (Soulseek's live count);
  - Soulseek: `.disc-folder` and `.disc-folder-tile` (a Soulseek folder), `.disc-dup` (a saved copy beside a Soulseek one), `.disc-kv` (source details);
  - sheets: `.disc-src` (the source picker sheet), `.disc-step` (a save step).

### Now Playing

- **The page:** `.now` (`data-tab`: `art` `lyrics` `queue` `details` on a phone; `data-page`: `now` `source` `save`), `data-sec` (which tab a section belongs to).
- **The hero:** `.now-head`, `.now-acts`, `.now-dev` and `.now-devc` (the hero and its compact form), `data-now` (live parts of a hero, section 6), `data-like`, `.now-slide`, `.now-osd`, `.now-sbtn` (a button on the device screen).
- **The cards:** `.now-ly` and `.now-lycard` (lyrics), `.now-q` and `.now-qcard` (the queue), `.now-next`, `.now-src` and `.now-srccard` (the source), `.now-seg` (the phone tabs).

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
  - structure: `.sys-settings`, `.sys-nav`, `.sys-page`, `.sys-section`, `.sys-card`, `.sys-group`, `.sys-row`, `.sys-grid`, `.sys-span`, `.sys-head`, `.sys-lede`;
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
- **Home and Search parts:** `.disc-agg`, `.disc-aggl`, `.disc-alt`, `.disc-alts`, `.disc-cat`, `.disc-cloud`, `.disc-col`, `.disc-fallback`, `.disc-issue`, `.disc-more`, `.disc-note`, `.disc-pending`, `.disc-picker`, `.disc-recover`, `.disc-shead`, `.disc-sheet`, `.disc-steps`, `.disc-sugg`, `.disc-tile-open`, `.disc-why`, `.disc-why-pop`, `.is-all`, `.is-better`, `.is-down`, `.is-nosource`, `.is-stale`, `.is-wait`.
- **Now Playing parts:** `.now-act`, `.now-fade`, `.is-denied`, `.is-far`, `.is-in`, `.is-lifted`, `.is-ready`, `.is-sorting`.
- **Library parts:** `.lib-adddesc`, `.lib-chips`, `.lib-cover`, `.lib-dl-storage`, `.lib-dropzone`, `.lib-facts`, `.lib-flist`, `.lib-foot`, `.lib-form`, `.lib-frow`, `.lib-help`, `.lib-hist`, `.lib-imp`, `.lib-import`, `.lib-kept`, `.lib-legend`, `.lib-link`, `.lib-link-card`, `.lib-linkrev`, `.lib-m3u`, `.lib-more`, `.lib-out`, `.lib-status`, `.lib-suggest`, `.lib-tagcard`, `.lib-tags`, `.lib-textarea`, `.lib-title`, `.lib-toggle`.
- **Settings and hub parts:**
  - Settings and Stats: `.sys-back`, `.sys-chips`, `.sys-combo`, `.sys-detail`, `.sys-follows`, `.sys-formats`, `.sys-funnel`, `.sys-health`, `.sys-hit`, `.sys-inline`, `.sys-keys`, `.sys-legend`, `.sys-locked`, `.sys-pending`, `.sys-meter-label`, `.sys-newart`, `.sys-nomatch`, `.sys-off`, `.sys-palette`, `.sys-plus`, `.sys-preview`, `.sys-range`, `.sys-rescan`, `.sys-seg`, `.sys-status`, `.sys-stepper`, `.sys-steps`, `.sys-taste`, `.sys-then`, `.sys-version`, `.sys-themes`;
  - hub and Connect: `.sys-cfg`, `.sys-connect`, `.sys-note`, `.hub-cred`, `.hub-dev`, `.hub-devs`, `.hub-form`, `.hub-jf`, `.hub-lib`, `.hub-qc`, `.hub-slsk`, `.cx-foot`, `.cx-list`, `.cx-note`, `.cx-title`;
  - states: `.has-section`, `.has-shot`, `.is-disabled`, `.is-index`, `.is-here`, `.is-online`;
  - Theme Studio: `.studio`, `.studio-sheet`, `.is-drop`, `.is-phone`.
- **Classic and desktop parts:** `.cls-root`, `.cls-slot`, `.cls-frame`, `.is-closed`, `.is-leaving`, `.dk-actions`, `.dk-choice`, `.dk-choices`, `.dk-code`, `.dk-details`, `.dk-dragstrip`, `.dk-hb`, `.dk-kv`, `.dk-sep`.
- **Attributes that carry ids and wiring, never looks:**
  - ids and keys: `data-id`, `data-ids`, `data-key`, `data-fk`, `data-uid`, `data-track-id`, `data-album-id`, `data-pl`, `data-qi`, `data-place-at`, `data-dl-mark`, `data-queue-index`, `data-si`, `data-pi`, `data-pic`, `data-wait-*`, `data-want-photo`, `data-i`, `data-t`, `data-d`, `data-q`, `data-ref`;
  - values and wiring: `data-kind`, `data-name`, `data-label`, `data-dir`, `data-pos`, `data-group`, `data-item`, `data-via`, `data-what`, `data-job`, `data-mode`, `data-choice`, `data-close`, `data-yes`, `data-form`, `data-url`, `data-text`, `data-msg`, `data-locked-msg`, `data-placeholder`, `data-cmd`, `data-device`, `data-remote`;
  - lists, filters and banners: `data-filters`, `data-issue`, `data-more`, `data-out`, `data-period`, `data-range`, `data-pick`, `data-pick-new`, `data-pick-made`, `data-pick-again`, `data-pick-done`, `data-sync`, `data-save-btn`, `data-save-songs`, `data-src-badge`, `data-src-mini`, `data-dl`, `data-cold-count`, `data-banner`, `data-banner-*`, `data-toast-*`, `data-cover-*`, `data-scan*`, `data-now-qlist`, `data-now-steer`, `data-fill` (a long list's row whose contents are drawn only near the screen: empty `<li>` while far from it);
  - Settings controls: `data-cfg`, `data-switch`, `data-step`, `data-stepper`, `data-min`, `data-max`, `data-unit`, `data-weight`, `data-when`, `data-format`, `data-formats`;
  - screens with their own wiring: `data-imp-*`, `data-lib-*`, `data-linkrev*`, `data-rev-*`, `data-cx*`, `data-studio*`, `data-drop-theme`, `data-theme-file`, `data-add-rates`, `data-skin`, `data-skin-file`, `data-webamp`, `data-eq*`, `data-token`, `data-token-reset`;
  - the desktop app's pages: `data-attempt`, `data-dk-attempt`, `data-dk-close`, `data-dk-connect`, `data-dk-error`, `data-dk-found`, `data-dk-offline`, `data-dk-retry-status`, `data-dk-welcome`.

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

