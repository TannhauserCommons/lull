# lull design spec

The specification for the web UI of lull (self-hosted music player: Soulseek streaming, Jellyfin saving, discovery,
Last.fm, Webamp Classic mode, PWA, Tauri wrapper). Read this first; the desktop wrapper is specified in
[desktop/DESKTOP.md](desktop/DESKTOP.md). Where this file and the code disagree, the code is right; fix this file.

Contents: 1 Principles · 2 The five designs · 3 Architecture · 4 Tokens · 5 Components · 6 Screens and states ·
7 App API · 8 Wiring to the server · 9 Motion · 10 Accessibility · 11 Responsive · 12 Classic mode (Webamp) ·
13 Media Session and PWA · 14 Do-nots

---

## 1. Principles

- **One app, five skins.** Every design shares the same markup, components, routes and behaviour. A design is a CSS
  layer (plus optional markup slots) chosen at runtime, without a reload. Never fork a screen per design.
- **Machine facts look like machine facts.** Formats, bitrates, sizes, times and service states use the design's
  "machine" face (pixel or mono, per design); everything a person reads uses the UI face.
- **Where the music comes from is always visible.** Saved (Jellyfin) vs Soulseek is shown by shape and icon, never
  colour alone, on every row, the player and the queue. A completed save flips the badge in place everywhere.
- **Streaming is honest.** The seek bar shows played, buffered ranges and the file's transfer progress; you can only
  seek inside what has arrived. Finding, queued, buffering, stalled-and-falling-back and failed are distinct states.
- **Nothing is dead.** Every visible control does something: an action, a sheet, a toast or a state. No "coming soon".
- **Calm by default.** One orchestrated moment per design (Now Playing opening), small functional motion elsewhere,
  all of it off under reduced motion.
- **Accessible in every design and theme:** text 4.5:1, large text and UI parts 3:1, visible focus, full keyboard use.

## 2. The five designs

Moodboard images are in [inspo/](inspo/) (01 starry playlist cards, 02 pastel widget desktop, 03 glassy aero player,
04 pink pixel playlist, 05 Y2K glossy window). No moodboard artwork is copied: every cover, ornament and illustration is
drawn in CSS or original SVG. Screenshots: [screenshots/](screenshots/).

| Design | Shell | Home theme | Extra fonts | Slots overridden |
|---|---|---|---|---|
| `lull` (default) | `topnav` | day | Audiowide | brand, coverArt, emptyArt, homeLead, nowHero |
| `skydeck` | `rail` | night | none (base Nunito + Jersey 10) | emptyArt, nowHero |
| `starlight` | `rail` | night | Jost, DM Mono | brand, coverArt, emptyArt, homeLead, nowHero, miniPlayer |
| `petal` | `topnav` | day | Nunito Sans | brand, coverArt, emptyArt, nowHero, miniPlayer |
| `glasshouse` | `dock` | night | Instrument Serif, IBM Plex Mono | brand, emptyArt, homeLead, nowHero |

"Home theme" is the theme the design was composed in first; both themes are complete. The app's theme pref defaults to
`system`.

**lull** (images 02 + 05, stacking from 01). A tidy Y2K pastel desktop: every view is a titled window with a glossy
lavender title bar and three decorative orbs, framed top and bottom by mocha trays; gloss only on things you press,
content flat and outlined. Night is a lavender-indigo Y2K night with the same windows. Signature moments: the "lull
pocket" Now Playing window (mocha screen, waveform, violet gloss play orb, speaker grill) laid out with a lyrics window
and a queue window that fill the desk; slanted indigo ribbons as section heads; tall "for you" tiles with pink reason
chips; the hatched "not downloaded yet" seek. Fonts: Audiowide for the wordmark and window titles, Nunito for the rest
(machine facts in Nunito 800, no pixel face).

**skydeck** (images 01 + 02). A pocket handheld under a dusk sky: cream cartridge cards stacked on periwinkle, the
player chrome built like a toy console (periwinkle body, mauve bezel, mocha LCD, glossy mauve keys), and a sky with a
moon, horizon glow and placed sparkles. The shared base styles are closest to this design; its layer finishes them. Signature moments: the Now
Playing handheld with a cartridge in its slot, power LED and D-pad; the player bar as the handheld turned sideways with
the seek inside an LCD strip; widget panels with periwinkle head bands. Fonts: Nunito (UI) and Jersey 10 (pixel face).

**starlight** (image 01). A starry-night poster: a slate sky of lit crumpled paper and fine stars, every surface a
cream tray stacked on lavender then periwinkle layers, burgundy caps titles, thin periwinkle line icons. Day is "dawn"
(lavender sky over a peach horizon). Signature moments: the Now Playing hero as image 01's top card (giant caps title,
outlined transport, framed art); eight hand-editable cover scenes (`designs/starlight/art/c1.svg` to `c8.svg`); "your queue as
a constellation" beside the Home greeting; a shooting star across the hero art per new song. Fonts: Jost (UI), DM Mono
(machine facts); the wordmark is SVG outlines (from Poiret One, OFL), so that font is not shipped.

**petal** (images 04 + 03). A pink pixel-art music zine: white pages framed by 1-2 px blush rules on a blush desk,
light poster titles with a pixel heart, uppercase micro-labels, hard pixel shadows and original pixel art
(`designs/petal/art/*.svg`, grid-aligned rects you can edit by hand). Night is wine-plum. Signature moments: the
Now Playing hero as a glossy pale "lull player" window (image 03) with a now / source / save tab strip; the playlist
page as image 04's table (# / title / album / date added / time, pink hearts); source scores as five pixel stars.
Fonts: Nunito Sans (UI), the base Jersey 10 for badges, keycaps and the LCD strip.

**glasshouse** (images 03 + 05 + 01). A silvery-glass Y2K media desktop after midnight: misty wallpaper with stars and
typewriter captions, frosted silver windows with aero title bars and glossy beads, desktop-icon navigation and a
status gadget column, one saturated anchor (a rose gloss play orb). Day is pale pearl. Translucency and blur appear
only under `@supports (backdrop-filter)`, over solid fallbacks that already pass contrast. Signature moments: the
Now Playing hero as an aero media-player window; a "now spinning" media window as the Home lead; the queue as a docked
gadget. Fonts: Instrument Serif (display, >= 1.25rem only), IBM Plex Mono (window titles, machine facts, captions),
Nunito (body).

## 3. Architecture

### 3.1 Files and load order

```
<head>  tokens.css, base.css, components.css, prototype/screens/*.css, desktop/desktop.css
        <link rel="stylesheet" id="design-css" href="../designs/lull/design.css">   (always last)
        <script src="../designs/designs.js"></script>
        <script> pre-paint: Designs.boot(); theme, motion, text size from stored prefs or the hash </script>
<body>  shell markup (rail, header, view, ctx panel, player bar, mini player, tab bar)
        icons.js, mock-data.js, app.js, screens/discover.js, now.js, library.js, system.js, classic.js,
        ../desktop/desktop.js
```

Layering: **tokens** (`tokens.css`) -> **surfaces** (`base.css`: `.s-sky .s-panel .s-card .s-screen` map theme
tokens to semantic ones) -> **components** (`components.css`, also the shell grid) -> **screens**
(`prototype/screens/<key>.css`, classes prefixed `.disc- .now- .lib- .sys- .cls-`) -> **design**
(`designs/<name>/design.css`, every rule scoped under `:root[data-design="<name>"]`).

Screens register routes, states, panels, actions and hooks at load time. `App` boots on `DOMContentLoaded`, waits for
the current design's `design.js` (if any) to register its slots (`Designs.ready`), then renders the shell and the
first route. The pre-paint script sets design, theme, motion and text size before first paint, so there is no flash.
The same order is used by [desktop/preview.html](desktop/preview.html) and [desktop/window.html](desktop/window.html).

Everything is classic scripts and plain CSS: no modules, no build step, no network. Port it into any stack; the class
names, attributes and the `App.*` seams are the contract.

### 3.2 Root and shell attributes

| Attribute | Values | Set by |
|---|---|---|
| `:root[data-design]` | `lull` `skydeck` `starlight` `petal` `glasshouse` | `Designs.boot` / `App.setDesign` |
| `:root[data-shell]` | `rail` `topnav` `dock` (from the registry) | same |
| `:root[data-theme]` | `day` `night`, always explicit ("system" is resolved and follows the OS live) | `Designs.applyTheme` |
| `:root[data-theme-pref]` | `day` `night` `system` (the user's choice) | same |
| `:root[data-motion]` | `reduce` `full` (absent = follow the OS) | prefs |
| `:root[data-classic="on"]` | Classic stage open | classic.js |
| `:root[data-titlebar]` | `win` `gnome` (desktop wrapper title bar in the app header) | desktop.js |
| `:root[data-window-inactive]` | wrapper window lost focus (caption glyphs dim) | desktop.js |
| `:root.is-hidden` | document hidden; loops pause | app.js |
| `.app[data-ctx]` | `docked` `open` `closed` (context panel) | app.js |
| `.app[data-view]` | `page` `now` (phone Now Playing sheet) | router |
| `.app[data-route]` | nav key: `home search library playlists stats settings` (empty otherwise) | router |
| `.app[data-screen]` | first path segment: `home search now library liked history following album artist tag playlists playlist stats settings welcome classic desktop` | router |
| `.main[data-title]`, `.main[data-meta]` | lowercase page title; the route's live meta line (`8 songs, 36 min, 250 MB`), for `content: attr(...)` | router, `App.setMeta` |

### 3.3 Design registry and switching

[designs/designs.js](designs/designs.js) defines `window.Designs`:

- `Designs.list.<name> = { label, shell: 'rail'|'topnav'|'dock', js: bool, preview: bool, description, swatches: [page, surface, accent, ink] }`
  (description and swatches feed the Settings > Appearance picker). `Designs.order` is the picker order;
  `Designs.fallback` = `'lull'`; `Designs.product` = `'lull'` (every title, brand label and copy reads it).
- `Designs.boot()` (pre-paint): design = `design=` in the hash, else the stored pref (`localStorage['lull:prefs'].design`),
  else the fallback; sets `data-design` / `data-shell`, points `#design-css` at `designs/<name>/design.css`, loads
  `design.js` when `js: true`.
- `App.setDesign(name, temporary?)` (also `App.setPref('design', name)`): loads the new stylesheet next to the old one,
  swaps on load, re-renders shell + route, emits `design`. If the stylesheet fails, the current design stays and a toast
  says so.
- `Designs.applyTheme('day'|'night'|'system')`: always writes an explicit `data-theme`, so designs never need a
  `prefers-color-scheme` block.

Keep the switcher in the real app (Settings > Appearance > Design). Every design's files load on demand only.

### 3.4 Writing `design.css`

Order inside the file: `@font-face`, tokens (a day block, then a `[data-theme="night"]` block), component selectors.

```css
:root[data-design="mine"] { --f-ui: "My Face", ui-rounded, sans-serif; --sky: #...; --panel: #...; }   /* day + invariant */
:root[data-design="mine"][data-theme="night"] { --sky: #...; --panel: #...; }                          /* night */
:root[data-design="mine"] .card { ... }
```

- Cascade gotcha: `:root[data-design=x]` has the same specificity as the base `:root[data-theme="night"]` and loads
  later, so a theme token set only in the day block also wins at night. Set every theme token you touch in both blocks.
- Cards read `--cream*` / `--ink*`; a design that wants dark or glass cards changes those (or restyles `.card`).
- Pseudo-elements are free for ornaments; keep ornaments decorative (`aria-hidden`, no hover, no cursor).
- Text on gradients or glass: check it against the darkest / lightest stop and record the pair as a comment,
  `/* check: --win-ink on --win-title 4.5 */` or `/* check: #ffffff on #8e7ce6 4.5 */`. Every design file carries these.
- Use `!important` only where the base writes a value inline (`--cover-size-base`, sky sparkle positions).

Hooks a design may target (stable in every screen, also after repaints):

| Hook | Notes |
|---|---|
| `[data-region]` = `rail header topnav content ctx playerbar mini tabbar lead` | shell regions (`lead` = Home's `homeLead` slot output) |
| `.region-title`, `.region-meta`, `#ctx[data-meta]` | hidden window titles for rail (`menu`), ctx panel (`queue`/`lyrics`/`activity` + meta) and player bar (`now playing`); show them to draw title bars |
| `.card-head` (first child of every `.card`), `.card-head__title`, `.card-head__meta`, `.card-head--hidden`, `.card[data-head="rule|hidden"]`, `.card[data-meta]` | every card has a head; cards without a visible label get a hidden one, so "every card is a window" is pure CSS |
| `.header__brand .header__nav .nav-pill .header__search .header__tools .header__shelf .header__settings`, `.rail__head .rail__nav .nav-item .rail__scroll .rail__tags .rail__list .rail__foot`, `[aria-current="page"]` | shell parts (current nav item) |
| `.tray__pill[data-svc="soulseek|jellyfin|lastfm|cache"][data-state="ok|warn|down"]`, `.tray__count`, `--cache-frac` (0-1), `data-cache-used`, `data-cache-cap`; compact pill: `[data-worst="ok|warn|down"]`, `.tray__word` (the worst state in words: "jellyfin down", "last.fm queued", "all good"), `.tray__gb` | status tray (e.g. a cache meter: `width: calc(var(--cache-frac) * 100%)`) |
| `li.trow[data-khz][data-kbps][data-bits][data-size]`, `.trow__fmt` | per-row file facts; `.trow__fmt` (`FLAC :: 44.1 kHz, 1123 kbps, 32.8 MB`) is hidden in the base |
| `.lib-hero__fmt`, `[data-tracks][data-duration][data-size]` on the album card | album file facts (hidden in the base) |
| `.banner.is-lead`, `.banner-more`, `.banner-slot.is-open` | phone banner collapse |
| `.sys-design[data-value="<name>"]` | Settings design picker cards |
| `#pbar` / `#mini` `[data-status]` (player status or `idle`), `[data-playing="true|false"]`, `--rx` (received %, integer 0-100: `counter-reset: rx var(--rx)` then `counter(rx) "%"`) | player bar and phone mini player state words |

### 3.5 Slots (`design.js`)

A design may replace a few pieces of markup. `design.js` is a classic script that only calls `Designs.define`:

```js
Designs.define('mine', {
  coverArt: function (ctx, fallback) { return String(fallback(ctx)).replace('class="cover ', 'class="cover mine-cover '); }
});
```

`Designs.slot(name, ctx, fallback)` (= `App.slot`) calls the override with `(ctx, fallback)`; it returns markup (string
or `App.html` value). A throwing override is logged and the fallback is used. Keep accessible names and the live hooks.

| Slot | Where | `ctx` | Default |
|---|---|---|---|
| `brand` | rail and header brand links (`[data-brand]`, labelled "lull home"), token gate, desktop pages, desktop board | `{ where: 'rail'|'header'|'desktop' }` | `Designs.brandMarkup()`: `.wordmark.brand > .brand__name` "lull" + `.brand__orn`; must read "lull" |
| `coverArt` | every `App.ui.cover` and each `.mosaic` cell | `{ key, recipe: 'c1'..'c8', size, cls, attrs, round, label, mosaic? }` | `<span class="cover cN" style="--cover-size-base:SIZE">` |
| `emptyArt` | `.empty__art` of every empty / error state | `{ art: 'moon'|'slot'|'star'|'wave', title }` | line drawing + sparkle |
| `homeLead` | Home, between greeting and first shelf, as `.disc-lead[data-region="lead"]` (only when it returns markup) | `{ track, player, queue (next 8), parts: { cover(t, size), playButton(size), likeButton(t), trackList(tracks, opts), formatLine(t) } }` | nothing |
| `nowHero` | Now Playing hero (`full`; `compact` on phone Lyrics/Queue/Details tabs and short landscape) | `{ variant, track, player, parts: { device(), compact(), screen(), playButton(size), likeButton(size), wave(bars), facts(), state() } }` | the handheld device |
| `miniPlayer` | desktop mini-player windows (runs without `App`: use only `ctx`, `window.icon`, `Designs`) | `{ style: 'device'|'strip', opaque, track { title, artist, format, lossless }, playing, liked, parts: { device(), strip(), cover(size), wave() } }` | device (340x560) or strip (540x90) |

Live hooks a custom hero must keep: `data-now="title|artist|album|format"` (text, filled on every track change),
`data-now="screen|c-screen|c-art|a|b|ca|cb|a-label|b-label|led|head|state|facts|repeat|osd|page"` (refreshed by now.js), and any
`[data-act="play|prev|next|shuffle|repeat|like"]`. `parts.playButton()` / `parts.likeButton()` are already wired. A hero
that drops the device screen must still show the source, format and streaming state: put `parts.facts()` (source badge,
quality chip, peer fact) in a `data-now="facts"` element and `parts.state()` in a `data-now="state"` element.
A custom mini player keeps `data-dk-player` on its root, `data-dk="play|prev|next|like|shuffle|repeat|seek|full|mini-close|vol-up|vol-down"`,
`data-dk-now`, `data-dk-pos`, `data-dk-dur`, and `data-tauri-drag-region` on the body (never on the seek strip).

### 3.6 Adding a design

1. Create `designs/<name>/design.css` (and optionally `design.js`, `fonts/` with an OFL file per family, art).
2. Add `Designs.list.<name>` and the name to `Designs.order` in `designs/designs.js` (`js: true` only if `design.js`
   exists; otherwise it is never requested).
3. Scope every rule under `:root[data-design="<name>"]`; set every theme token in both day and night blocks.
4. Add picker previews as `assets/previews/<name>-{day,night}.webp` and set `preview: true` (without them the card
   draws the shell in your swatches).
5. Check every route and the states in section 6, day and night, desktop and phone; contrast; focus; `motion=reduce`.

## 4. Tokens

Values live in [tokens.css](tokens.css) (base) and each `design.css`. Components never read theme tokens directly:
they read the **semantic surface tokens** set by the surface classes.

| Semantic (read by components) | Meaning |
|---|---|
| `--bg` `--bg-2` `--bg-3` | surface fill, hover, pressed |
| `--fg` `--fg-2` | text, secondary text (both >= 4.5:1 on `--bg`) |
| `--link` `--accent-ui` | links and the accent for selected / primary UI |
| `--edge` `--divider` | control outlines (>= 3:1) and hairlines |
| `--focus` | the focus ring (>= 3:1 on the surface) |
| `--heart` | liked heart |
| `--q-ll` `--q-ly` | lossless / lossy quality chips |
| `--rsn-bg` `--rsn-fg` | discovery reason chips |
| `--seek-played` `--seek-buffered` `--seek-line` | seek bar layers |

Surfaces: `.s-sky` (page, headings on the page), `.s-panel` (rail, player bar, ctx panel, inputs, tab bar), `.s-card`
/ `.card` (cards, sheets, dialogs, popovers, toasts), `.s-screen` (device screen). A `.s-panel` or `.s-sky` nested in a
card gets the theme values back (`--theme-*`).

| Family | Tokens |
|---|---|
| theme (day / night) | `--sky --sky-deep --panel --panel-2 --tray --tray-edge --fg-sky --fg-sky-2 --fg-panel --fg-panel-2 --link --accent-ui --heart --edge --focus --seek-played --seek-buffered --seek-line --q-lossless --q-lossy --reason-bg --reason-fg --play-ring --wordmark-2 --sparkle --speck --scrim --scrim-alpha --gloss-ring-w` |
| palette (theme-invariant) | `--cream* --ink* --link-on-cream --peri --peri-deep --lav --lavender --lilac --indigo --star --pink* --mauve --mocha --mocha-edge --screen --line --stack-1 --stack-2 --ok --warn --down --skel --skel-hi --wave-dim --dusk --dusk-deep --card-seek-* --win-*` |
| type | families `--f-ui` (UI) and `--f-px` (machine facts); scale `--t-display --t-title-xl --t-h2 --t-row --t-body --t-meta --t-micro --t-px --t-lyric --t-lyric-cur` (font shorthands) + `--t-*-track`; classes `.t-*` in base.css |
| space | `--sp-1..12` (rem, follow text size), `--gutter` (32 / 24 / 16), `--card-pad` |
| shape | `--r-chip --r-thumb --r-art --r-input --r-panel --r-card --r-pill --r-device --r-bezel`, `--stroke --stroke-thin --icon-stroke` |
| depth | `--stack --stack-sm --stack-1l --stack-press --stack-reserve` (offset paper layers, not blur) |
| motion | `--dur-press --dur-1..4`, `--ease-out --ease-in-out --ease-spring` |
| layers | `--z-bar 20 --z-panel 30 --z-classic 35 --z-sheet 40 --z-popover 50 --z-toast 60 --z-overlay 80` |
| sizes | `--rail --rail-icons --ctx --bar-h --mini-h --tab-h --hit` (44 px minimum hit target) |

Status colours (`--ok --warn --down`) always come with a glyph shape and a label.

**Font coverage.** Every shipped `.woff2` is the Google Fonts *latin* subset. Soulseek metadata often carries latin-ext
(ł, ő, ş), Cyrillic, Greek or CJK; those glyphs fall back to the next family in each stack (`ui-sans-serif`,
`system-ui`, `ui-monospace`), so a title can switch typeface mid-word. Before release, add each UI family's latin-ext
(and, where the family has them, cyrillic / greek) subset as a second `@font-face` with the same family name and a
`unicode-range` (as Google Fonts' CSS does); CJK stays on the system font.

## 5. Components

All in [components.css](components.css); render them with the `App.ui.*` helpers (section 7.2) where one exists.
Icons: [icons.js](icons.js) defines `icon(name, label?, cls?)` (an inline `<svg class="ic">` using the injected sprite;
a label makes it `role="img"`) and `ICON_NAMES`. Sizes `.ic` 20 px, `.ic-16`, `.ic-24`.

| Group | Classes and states |
|---|---|
| Buttons | `.btn` (secondary) `--primary` (one per region) `--quiet` `--danger` `--sm` `--block`; disabled via `disabled` / `aria-disabled`. `.btn-icon` `--bare` `--sm`, `[aria-pressed="true"]`, `.is-liked`, `.btn-icon__badge`. `.btn-gloss` (main play, device A/B only) `--64 --56 --52 --48 --40`, `.is-buffering` + `.btn-gloss__spin`, `.is-liked`, `.ic-play` (optical centring) |
| Chips | `.chip` (`[aria-pressed]`, `.chip__check`), `.chip-reason` + `.chip-reason__name`, `.chip-more`, `.pill-filter[aria-pressed]` |
| Badges | `.badge.badge-saved`, `.badge.badge-slsk`, `.q-chip--ll` / `--ly` (+ `--solid` in player and device), `.badge-new`, `.badge-count`, `.dup-note`, `.dup-better`, `.resolve`, `.spin-ring`, `.scrobble--done|pending|short`, `.sg--ok|warn|down` |
| Cards | `.card` (`--flush`), `.label-rule` (`--sm`), `.inset` (flat group inside a card) |
| Tiles, covers | `.shelf` (scroll-snap row), `.tile` (`__art __play __title __by __meta`); `.cover.c1..c8` (`--thumb --xs --round`, size `--cover-size`), `.mosaic` (2x2) |
| Track rows | `ol.tlist` (`--dense --dated`, a container query) > `li.trow` (`__num __grip __art __main __title __artist __album __added __badges __sources __like __dur __more __fmt`); `.is-playing .is-playing-now .is-selected .is-unavailable .is-failed .is-dragging`; `.eq` (playing bars) |
| Source picker | `.src-table` > `.src-row` (`--head`, `.is-failed`, `.is-preferred`), `.score` (`--s` 0-1) |
| Seek, waveform, volume | `.seek` (`__track __xfer __bufs __buf __played __rail __knob __tip`; `--pos`, `--xfer`; `.is-transferring .is-dragging .is-shake`), `.seek--line`, `.seek-row`; `.wave` (`__bar` with `--h`, `.is-played .is-buf .is-xfer`, `__rail __head`), `.wave-times`; `.vol`, `.range` (`--fill`) |
| Inputs | `.input` (`--readonly`), `.field` (`__label __help __error`), `.search-field` (`__end __clear`, `.kbd`), `button.switch[role=switch][aria-checked]` in `label.switch-row` |
| Tabs | `.seg[role=tablist]` (`--n`, `--i`) + `.seg__ind` + `.seg__btn[role=tab][aria-selected]` (arrow keys handled by the owner) |
| Overlays | `.sheet .dialog` (`__head __title __body __foot`), `.scrim`, `.popover` (`__head`), `.menu .menu__item .menu__sep`, `.tooltip` (`data-tip`), `.toast`, `.banner` (`.banner-slot`) |
| Status | `.tray` (`--full --compact`), `.tray__pill`, `.status-list .status-row` |
| Device | `.device` (`.is-power-on`, `--device-w`) with `__led __bezel __screen __page __now __title __artist __facts __brand __controls __ab __key --a|--b __pills __grill`, `.dpad`, `.device--compact`, `.bezel-thumb` |
| Save, jobs | `.source-card`, `.pipeline` > `.pipeline__seg.is-done|is-current|is-failed|is-skipped`, `.btn-save[data-state="idle|saving|saved|failed|library|pending"]` (`--p`), `.progress` (`--xfer --failed`) |
| Loading, empty | `.skel` (`--text --title --chip --cover`), `.skel-row`, `.skel-tile` (appear after 150 ms); `.empty` (`__art __title __text __actions`) |
| Misc | `.kbd`, `.shortcut-list .shortcut-row`, `.sparkle` (decorative, `--s`), `.wordmark`, `.skip-link` |
| Classic chrome | `.win` (`__bar __title __orbs`), `.orb--blue|lav|mag` |
| Drag and drop | `.drag-chip`, `.is-drop-target`, `.drop-line`, `.drop-overlay` |
| Shell | `.app` grid; `.rail`, `.header`, `.main > #view.view`, `#ctx.ctx`, `#pbar.pbar` (`__now __centre __controls __seek __tools __state`), `#mini.mini`, `.tabbar > .tab`, `.ctx-scrim` |

## 6. Screens and states

Open any state with `prototype/index.html#<route>?state=<id>` (or the "states" button, bottom right, which lists and
filters all of them, about 170). Combine with `&design=<name>&theme=day|night`. State ids are defined in each screen's
`App.registerStates(...)` list; demo branches in screen code carry a `PROTO:` comment.

| Screen | Route | State ids |
|---|---|---|
| Player bar, mini player (every route) | any | `player-idle player-finding player-queued player-buffering player-streaming player-stalled player-failed player-library player-paused player-ended player-jellyfin-error player-crossfade` |
| Service banners | any | `system-banner-unreachable system-banner-offline system-banner-jellyfin system-banner-lastfm system-banner-soulseek system-banner-cache system-banner-cache-warn system-banner-all` |
| Token gate, first-run connect (in the web UI) | any | `system-gate system-gate-rejected system-desktop-connect system-desktop-testing system-desktop-unreachable` |
| Shortcuts overlay, command palette | any | `system-shortcuts system-shortcuts-filter system-palette system-palette-query` |
| PWA | any | `system-install-toast system-update-toast system-ios-sheet` |
| Welcome (first run) | `/welcome` | `system-welcome` |
| Home / discovery | `/home` | `discover-home discover-home-loading discover-home-shelf-error discover-home-cold discover-home-no-source discover-home-reason discover-radio` |
| Search | `/search?q=` | `discover-search-empty discover-search-typing discover-search-results discover-search-dup discover-search-none discover-search-slsk-down` |
| Soulseek album folder, source picker, source details (sheets) | `/search`, `/home` | `discover-folder discover-picker discover-picker-stalled discover-details discover-details-fallback` |
| Save flow (duplicate dialog, 5-step pipeline, failures, release picker, aggregates) | `/search`, `/home` | `discover-save-duplicate discover-save-progress discover-save-fail-transfer discover-save-fail-match discover-save-release-picker discover-save-no-cover discover-save-fail-write discover-save-pending discover-save-album discover-save-queue` |
| Now Playing | `/now?tab=art|lyrics|queue|details&track=` | `now-streaming now-library now-paused now-page-source now-page-save now-save-failed now-finding now-stalled now-failed now-idle now-phone-details now-phone-queue` |
| Lyrics (Now tab and ctx panel) | `/now` | `now-lyrics-word now-lyrics-synced now-lyrics-scrolled now-lyrics-plain now-lyrics-instrumental now-lyrics-notfound now-lyrics-loading now-lyrics-error` |
| Queue panel | `/now` | `now-queue-radio now-queue-user now-queue-radio-error now-queue-empty` |
| Activity panel (transfers, saves, imports, rescans) | `/now` | `now-activity now-activity-failed now-activity-quiet` |
| Library | `/library/albums|artists|songs|genres` | `library-albums library-artists library-songs library-genres library-empty library-jellyfin-down` |
| Album | `/album/:id` | `library-album-saved library-album-partial library-album-soulseek library-album-saving library-album-failed` |
| Artist, tag | `/artist/:id`, `/tag/:id` | `library-artist library-artist-unfollowed library-tag` |
| Liked, history, following | `/liked`, `/history`, `/following` | `library-liked library-liked-empty library-history library-history-empty library-following library-following-empty` |
| Playlists, M3U import | `/playlists` | `library-playlists library-playlists-empty library-playlist-create library-playlist-deleted library-m3u-start library-m3u-preview library-m3u-error` |
| Playlist, add-to-playlist picker | `/playlist/:id`, `/album/:id` | `library-playlist library-playlist-sorted library-playlist-reorder library-playlist-empty library-playlist-drop library-playlist-edit library-playlist-export library-playlist-picker` |
| Stats | `/stats?period=7d|30d|12m|all` | `system-stats system-stats-12m system-stats-loading system-stats-empty system-stats-reset-confirm system-stats-reset` |
| Settings | `/settings/:section` (`playback sources cache library sharing lastfm discovery appearance shortcuts app`) | `system-settings-index` (phone list) `system-settings-playback system-settings-sources system-cache-full system-settings-library system-naming-error system-sharing-none system-lastfm-off system-lastfm-waiting system-lastfm-connected system-lastfm-error system-settings-discovery system-settings-appearance system-settings-shortcuts system-settings-app system-settings-app-installed system-settings-app-desktop` |
| Classic mode | `/classic` | `classic-stage classic-drawer-closed classic-favourites classic-favourites-empty classic-downloaded classic-search-empty classic-loading classic-preview classic-applying classic-failed classic-offline classic-nsfw classic-notice classic-drop` |
| Desktop wrapper pages | `/desktop` (only when `desktop.js` is loaded) | `desktop-connect-idle desktop-connect-testing desktop-connect-ok desktop-connect-unreachable desktop-connect-rejected desktop-connect-invalid desktop-offline desktop-titlebar-win desktop-titlebar-gnome desktop-close-dialog` |

Hash flags (any route; applied on navigation, not stored): `theme=day|night|system`, `motion=reduce|full|system`,
`design=<name>`, `titlebar=win|gnome`, and prototype-only `state=<id>`, `switcher=0` (hide the states button),
`clock=HH` (pins the Home greeting hour). Example: `#/album/al-lantern-season?theme=night&design=petal`.

Behaviour worth knowing per screen (not obvious from a screenshot):

- **Search** shows library results instantly, then Soulseek results stream in with a live counter (results, peers) for
  10-20 s. Songs merge across peers (one row, "N sources"); Soulseek folders group as albums. A Soulseek row that
  matches a saved song carries `dupNote` / `dupBetter`.
- **Source picker** ranks by score (format, speed, free slot, queue; tooltip breaks it down), marks the preferred
  format, greys sources that stalled earlier, and keeps loading while the search runs. Picking a source plays it.
- **Save** runs `hooks.beforeSave` (duplicate dialog: keep / save as a separate version / replace if better), then five
  steps: completing transfer, MusicBrainz match, cover art, tag + write, Jellyfin rescan. Missing cover art does not
  block; a match failure offers the release picker or "save with existing tags"; a Jellyfin failure leaves the file
  saved and pending. Albums and the queue show aggregate progress.
- **Queue** groups: now playing, next up (the listener's own picks: "Play next" at its front, "Add to queue" at its
  end), then (context), radio tail ("keeps adding" toggle + familiar/adventurous steer). Next up plays before the
  context continues, survives starting another album, playlist or radio, and plays in order under shuffle; shuffle
  then plays every other song once per round (repeat all starts a new round). Each item shows source and resolve
  state (ready / resolving / none).
- **Lyrics** auto-scroll pauses when the user scrolls ("back to current line" pill); clicking a line seeks only inside
  what is buffered; attribution "Lyrics provided by LRCLIB".
- **Settings**: values that come from server config (credentials, URLs, keys) are read-only with "set in server
  config"; never editable or revealed in the UI. Settings > App items for the wrapper show only inside it.

## 7. App API (prototype core)

[prototype/app.js](prototype/app.js) defines `window.App` (sections: 1 utilities, 2 events + store, 3 prefs + designs,
4 router + states, 5 render helpers, 6 overlays, 7 save pipeline, 8 player engine, 9 live bindings, 10 shell,
11 actions, 12 keyboard, 13 Media Session, 14 drag and drop, 15 boot). Screens are IIFEs:
`(function () { var html = App.html, ui = App.ui, M = App.data; ... })();`

### 7.1 Router

```js
App.route('/album/:id', { render: function (ctx) { return html`...`; }, mount: fn, title: 'Album' | fn(ctx),
  nav: 'library', view: 'page' | 'now', meta: fn(ctx), keepFocus: bool });
```
- Hash routes, `/x/:param`, optional `/:param?`, query after `?`. `ctx = { params, query, state, el, route, onCleanup(fn) }`;
  `ctx.el` is `#view` (the scroll container). `render` returns markup; `mount` runs after insertion; `onCleanup` runs
  before the next route.
- `App.go(path, { replace })`, `App.back()`, `App.refresh()` (re-render, no scroll reset), `App.current`
  (`{ path, params, query, state, route }`), `App.setMeta(text)`.
- `view: 'now'` shows the route as a full-height sheet on phones. Document title = `<title> - lull`.
- Routes: `/home /search /now /library/:segment? /album/:id /artist/:id /tag/:id /liked /history /following /playlists
  /playlist/:id /stats /settings/:section? /welcome /classic` (+ `/desktop` from desktop.js).

### 7.2 Markup helpers

- ``App.html`...` `` escapes interpolated strings; values from `App.html`, `App.raw`, `App.icon`, `App.ui.*` (and
  arrays of them) are inserted raw. Also `App.raw`, `App.esc`, `App.icon(name, label?, cls?)`, `App.$`, `App.$$`,
  `App.uid`, `App.clamp`, `App.coverClass(key)`, `App.storage(key, value?)` (localStorage JSON under `lull:`, wrapped in
  try/catch; per-device conveniences only).
- `App.fmt`: `time(sec, estimated)` `3:52` / `~3:52`, `date(iso, withYear)`, `length(sec)` `48 min`, `size(mb)`,
  `clock(sec)` `00:36:12`, `formatLine(track, { bits })` `FLAC :: 44.1 kHz, 1123 kbps, 32.8 MB`, `albumLine(album)`.
- `App.ui.*` (all return safe markup): `cover(itemOrId, size, { round, label, cls })`, `mosaic(ids, size, label)`,
  `trackRow(trackOrId, opts)`, `trackList(tracks, { label, dated, added, numbered, dense, showSources, dup, actions })`,
  `releaseTile(album, { showNew })`, `sourceBadge`, `savedBadge`, `soulseekBadge`, `qualityChip(label, { solid, button, id })`,
  `newBadge`, `countBadge`, `keycap`, `statusGlyph`, `dupNote`, `dupBetter`, `resolve`, `scrobble`, `tagChip`, `chip`,
  `filterPill`, `reasonChip(reason, { more, via, itemId })`, `labelRule(text, { action, level, sm, id })`,
  `iconButton(icon, label, attrs)`, `likeButton`, `seekBar({ bind: 'player'|'volume' })`, `volume`, `wave(trackId, bars)`,
  `waveTimes`, `saveButton(trackId)`, `pipeline(step, status, error)`, `emptyState({ art, title, text, action, secondary })`,
  `skeleton(kind, n)`, `sparkle`, `wordmark(where)`, `brand(where)`, `cardHeads(root)`.
- Live parts stay in sync by themselves: like buttons, save buttons, source badges, seek bars, waveforms, playing rows,
  and any `data-bind="pos|dur|buffered-to|received"` text. After injecting rows outside a route render, call `App.syncRows()`.

### 7.3 Actions

Any element with `data-act="<name>"` runs that action on click (`data-id` / `data-ids` carry ids; `data-close` also
closes the top sheet). `App.action(name, fn(el, event))` adds or replaces one. Built in: `play-track play-album
shuffle-album play-playlist play-ids queue-next queue-add like save save-album go-album follow follow-tag start-radio
start-radio-current add-to-playlist show-sources pick-source source-details reason more-like less-like hide-artist
track-menu player-more play next prev shuffle repeat mute retry-search try-soulseek toggle-panel close-panel
open-activity open-now theme status more-sheet shortcuts palette classic soulseek-reconnect jellyfin-rescan
lastfm-retry open-cache back noop reload`. Screens add their own, prefixed (`disc-*`, `lib-*`, `sys-*`, `cls-*`,
`now-*`).

### 7.4 Overlays

`App.openSheet({ title, body, foot, wide, className, label, role, closable, dismissible, onMount(el, close), onClose })`
and `App.openDialog(same)` (focus trap, Esc, focus returns to the opener; phone sheets drag down to dismiss; dialogs
never stack), `App.closeTop()`, `App.confirm({ title, text, confirm, danger }, onYes)`,
`App.popover(anchor, markup, opts)` / `App.closePopover()`, `App.menu(anchor, items, { label })`, `data-tip="..."`
tooltips, `App.toast(msg, { icon, kind: 'info'|'ok'|'error', action, onAction, sticky })`,
`App.banner.show({ id, kind: 'warn'|'down'|'info'|'ok', icon, text, action: { label, run }, dismissible })`, `.hide(id)`,
`.clear()`, `.list()`, `App.announce(text)` (polite live region). Escape order: popover, sheet / dialog, ctx overlay,
phone Now Playing.

### 7.5 Store, events, prefs

- `App.store.get(key)`, `.set(key, value)` / `.set({...})`, `.update(key, fn)`, `.subscribe(fn(keys))`. Keys: `liked`
  `{id: true}`, `followed` `{artistId|tagId: true}`, `saves` `{trackId: { step 1-5, status: 'saving'|'saved'|'failed'|'pending', pct, error, kind, frozen }}`,
  `services`, `activity`, `playlists`, `recentSearches`, `hiddenArtists`, `ctxTab`, `crossfadeSec`.
- Helpers: `App.isLiked`, `App.toggleLike(id, force?)`, `App.isFollowed`, `App.toggleFollow`, `App.isSaved`,
  `App.track(id)`, `App.setService(key, patch)` (repaints the tray), `App.activeJobs()`, `App.addToPlaylist(plId, ids)`.
- Events (`App.on(name, fn)` returns an unsubscribe; `App.emit`): `route`, `player` (kind: `track play seek mode volume
  queue status source ended state`), `tick` (every 250 ms while a track is loaded: position, transfer), `store`, `like`, `follow`, `save`
  (`{ id, save }`), `prefs`, `design`, `states`, `seek-denied`, `row:move`, `row:delete`, `drop`, `ready`.
- Prefs (`localStorage['lull:prefs']`): `App.prefs()` -> `{ theme, motion, textSize, design }`;
  `App.setPref('theme', 'day'|'night'|null)`, `('motion', 'reduce'|'full'|'system')`, `('textSize', 90|100|112|125)`,
  `('design', name)`; `App.theme()`, `App.toggleTheme()`, `App.reducedMotion()`, `App.design()`, `App.designs`,
  `App.designOrder`, `App.product`.

### 7.6 Hooks

Replace one to take over a flow; the defaults keep every control working. `App.hooks.beforeSave(ids, proceed)` (call
`proceed(idsToSave)`), `sourcePicker(trackId)`, `sourceDetails()`, `reason(anchor, reason)`, `addToPlaylist(ids,
anchor)`, `shortcuts()`, `palette()`, `importFile(file)` (M3U or `.wsz` dropped on the window; classic.js wraps the
library handler), `moreSheet()` (phone More tab). The screens install their own versions of most of these.
Screen helpers are also exposed: `App.discover.{openFolder, openPicker, openDetails, openSaveSheet, openAggregate,
openReleasePicker, openDuplicate, startRadio}`.

### 7.7 Context panel

`App.panel(id, { label, render(bodyEl), mount(bodyEl) })` registers a tab (`queue`, `lyrics`, `activity`);
`App.openPanel(tab)`, `App.closePanel()`, `App.togglePanel(tab)`, `App.refreshPanel()`. It re-renders on queue / track
changes (Queue, Lyrics) and on `saves` / `activity` changes (Activity). The panel body is a queue drop target.

### 7.8 Player engine (`App.player`)

State (`App.player.state`): `queue [{ uid, trackId, group: 'next'|'context'|'radio', resolve: 'ready'|'resolved'|'resolving'|'none' }]`,
`index`, `playing`, `pos` (s), `status` (`idle finding queued buffering streaming stalled failed library ended
jellyfin-error`; paused = `playing: false` while streaming or library), `received` (0-1 of the file transferred),
`buffered [[from, to], ...]` (fractions), `durationKnown`, `peer { name, rank, of, speed, format, sizeMB }`,
`peersFound`, `queuePos`, `fallback { from, to, n, of }`, `failedPeers [{ peer, at }]`, `crossfade`, `shuffle`,
`repeat 'off'|'all'|'one'`, `volume` 0-1, `muted`, `radio { name, seed, keepsAdding, steer } | null`, `error`.

Methods: `current() currentId() duration() upNext(n) play() pause() toggle() next() prev() seek(sec) seekBy(sec)
seekFraction(f)` (returns false and shakes outside buffered) `setShuffle(on?) cycleRepeat() setVolume(v) toggleMute()
playList(ids, start, { radio }) playTrack(id, contextIds?) playNext(ids) addToQueue(ids)` (before the radio tail)
`removeAt(i)` (undo toast) `move(from, to) clear()` (undo toast) `startRadio({ kind, id }) setRadio(patch|null)
useSource(trackId, sourceIndex) retry() statusText()`; prototype-only: `stall()`, `setState(patch)`.

The shell renders from this state alone: player bar (every status), mini player, phone Now Playing, Media Session,
seek bars and waveforms. Keep the state shape and the `player` / `tick` events and everything follows.

### 7.9 Save pipeline (`App.saves`)

`start(ids)` (skips saved and in-flight ids, runs `hooks.beforeSave`), `retry(id)`, `get(id)`, `set(id, record|null)`.
Records `{ step 1-5, status, pct, error, kind: 'transfer'|'match'|'write', nocover, frozen }`; every change emits
`save` and re-syncs rows, badges and the Activity count. Step labels: `App.data.saveSteps`.

### 7.10 Mock data shape (`window.MOCK` = `App.data`)

All fictional. Ids are stable strings: `ar-*` artists, `al-*` albums, `t-<album>-<n>` tracks, `pl-*` playlists, tag
id = tag name.

- `formats { 'FLAC 16/44.1': { codec, label, detail, lossless, mbPerMin, kbps, khz, bits }, ... }`, `saveSteps [5]`.
- `tags [{ id, name, desc, followed, related[] }]`; `artists [{ id, name, tags[], followed, listeners, bio }]`.
- `albums [{ id, title, artistId, artist, year, type: 'album'|'EP'|'single', released, label, format, tags[], isNew, saved: 'all'|'partial'|'none', trackIds[], duration, sizeMB }]`.
- `tracks [{ id, title, artistId, artist, albumId, album, year, no, disc, duration, format, khz, bits, kbps, sizeMB, saved, liked, plays, tags[], sources [{ peer, format, sizeMB, speed, freeSlot, queue, score 0-100, factors { format, speed, slot, queue }, path }] }]` (sources best first).
- `feed { greeting, updated, summary, newReleases [albumId], picks [{ trackId, reasons [{ kind: 'like'|'tag'|'listeners'|'fresh', name, ref }], via, noSource? }], radioStarters [{ id, title, sub, seed { kind, id } }], jumpBackIn [{ kind, id }], followedTags [], coldStartTags [] }`.
- `playlists [{ id, name, desc, created, updated, items [{ trackId, added }] }]`; `history [{ day, items [{ trackId, at, scrobble: 'done'|'pending'|'short' }] }]`.
- `stats { periods, byPeriod { '7d': { minutes, plays, artists, saves, skipRate, newArtists, hitRate { recommended, liked, saved }, topArtists, topTracks, topTags, clock [24] } }, taste { artists [[id, weight]], tags [[tag, weight]] } }`.
- `lyrics { trackId: { instrumental, attribution, syncedLyrics (LRC), plainLyrics?, wordSync? } }` (other tracks = not found).
- `activity [{ id, kind: 'transfer'|'save'|'import'|'rescan', trackId?, peer?, speed?, pct?, state?, step?, stepState?, error?, started }]`.
- `services { soulseek|jellyfin|lastfm|cache: { state: 'ok'|'warn'|'down', label, detail, queued?, usedGB?, capGB? } }`.
- `skins [{ id (md5), name, file, palette [3], favourite, downloaded, nsfw }]`; `settings { playback, sources, cache, library, sharing, lastfm, discovery, appearance, app }`.
- `recentSearches []`, `queue { radio, items }`, `peers []`; lookups `track(id) album(id) artist(id) tag(id) playlist(id) albumsBy(artistId) tracksBy(artistId)`.

## 8. Wiring to the server

**Keep:** tokens, base, components, `designs/`, icons, fonts, assets, the shell markup in `prototype/index.html`,
`app.js` sections 1-6 and 9-15, the screens (minus `PROTO:` branches), `desktop/desktop.css` + `desktop.js`.
**Replace:** `mock-data.js`, the insides of `app.js` sections 7 (saves) and 8 (player), every `PROTO:` simulated delay.
**Remove:** the states switcher (system.js section 10), every `App.registerStates` list, `ctx.state` branches, the
`?state=` / `switcher` / `clock` flags, and the design board (`desktop/preview.*` and `desktop/window.html`; window.html
renders its views through preview.js, so it is board-only too). The real mini window is its own page that loads tokens,
base, components, `desktop.css`, the design, `icons.js` and `desktop.js`, puts `LullDesktop.miniDevice()` or
`miniStrip()` in the body and calls `LullDesktop.bind(document)`; it and the bundled connect / unreachable pages get
the user's design and theme from the wrapper in the URL hash (`#design=<name>&theme=<t>`, see DESKTOP.md 6.1). `grep -rn "PROTO" prototype desktop` lists every
place (in desktop.js: the mock connect test, the mock mini player and section 7).

Screens read `App.data` **synchronously** (`M.track(id)`, `M.album(id)`, `M.feed`). The least invasive port: a client
cache with the same object shapes and the same lookup functions, filled per route before render (or render the
skeleton, fetch, then `App.refresh()`), plus a server event stream that drives the player, saves, jobs and services.

Endpoint names below are suggestions; the shapes are what the UI needs.

| Prototype (mock) | Used by | Server side |
|---|---|---|
| `M.track/album/artist/tag/playlist`, `albumsBy`, `tracksBy` | every screen | library + Soulseek metadata by stable id: `GET /api/tracks/:id`, `/api/albums/:id`, `/api/artists/:id` (top tracks, discography, similar), `/api/tags/:id` (description, top artists, new releases, related) |
| `M.albums`, `M.tracks` filtered | Library segments | `GET /api/library/{albums,artists,songs,genres}?sort=&filter=` (Jellyfin-backed; cached read-only list when Jellyfin is down) |
| `M.feed` | Home | `GET /api/feed` (new releases from follows, picks with `reasons` + `via` source, radio starters, jump back in, followed / cold-start tags, `updated`); per-shelf errors |
| simulated search (`searchTick`) | Search, palette | `GET /api/search?q=` returns library hits at once; Soulseek results stream (SSE / WebSocket) as merged songs + folder albums with `{ results, peers, done }` counters |
| `track.sources`, picker loading | source picker, album folder sheet | `GET /api/tracks/:id/sources` ranked with `score` + `factors`, streaming while the search runs; failed / stalled peers flagged |
| `App.player` timer | player bar, Now Playing, Media Session | `<audio>` on `GET /api/stream/:trackId[?source=]` (Range requests; Jellyfin for saved tracks); stream events for `status`, `peer`, `peersFound`, `queuePos`, `received`, `fallback`, `failedPeers`; `buffered` from `audio.buffered` intersected with `received` |
| radio top-up | queue | `GET /api/radio?kind=&id=&steer=` (next tracks) |
| `App.saves` timer, release picker | save everywhere, Activity | `POST /api/saves { ids }`, progress events `{ id, step, status, pct, error, kind, nocover }`, `GET /api/saves/:id/releases`, `POST /api/saves/:id/release`, retry |
| default `hooks.beforeSave` + duplicate dialog | save | `POST /api/library/duplicates { ids }` -> existing file + quality comparison; policy from settings |
| `M.activity` / store `activity` | Activity panel, badge | `GET /api/jobs` + job events (transfers, saves, imports, rescans); cancel / retry / clear finished |
| `M.services` / `App.setService` | status tray, banners | `GET /health` + service events: Soulseek (connected / reconnecting / down), Jellyfin, Last.fm (`queued` scrobbles), cache (`usedGB`, `capGB`) |
| store `liked`, `followed`, `hiddenArtists` | everywhere | `PUT/DELETE /api/likes/:id`, `/api/follows/:id`; `POST /api/feedback { more-like | less-like | hide-artist }` |
| store `playlists` | playlists, picker, rail | `/api/playlists` CRUD + reorder; `POST /api/playlists/import` (M3U -> per-line match preview: library / will search Soulseek / not found), `GET /api/playlists/:id.m3u8` |
| `M.history` | History | `GET /api/history` grouped by day with scrobble status |
| `M.stats` | Stats | `GET /api/stats?period=7d|30d|12m|all`; `POST /api/taste/reset` |
| `M.lyrics` | Lyrics | `GET /api/lyrics/:trackId` (LRCLIB, fetched and cached server-side): synced / word-synced / plain / instrumental / not found / rate-limited |
| `M.settings` | Settings | `GET/PATCH /api/settings`; config-sourced values come back read-only; Jellyfin test + rescan; Last.fm auth URL, callback, disconnect, retry scrobbles; cache clear |
| `M.skins`, museum search delay | Settings > Appearance > Classic | museum GraphQL (direct or proxied), `GET /api/skins/:md5.wsz` (server-cached, same origin), favourites stored server-side |
| cover recipes (`.c1-.c8`) | every cover, Media Session | `GET /api/art/:albumId?size=96|256|512` (same origin); see the note below |
| token gate mock | gate | shared token check (`GET /api/ping` -> 401) |
| Settings > App health / update | App section | `GET /health`; service worker update flow |

**Real cover art.** The `coverArt` slot renders a `<span class="cover cN">` whose recipe draws placeholder art in CSS.
For real art, put an `<img class="cover__img" loading="lazy" alt="">` inside that span from the default
`coverMarkup` in app.js (the designs' overrides all wrap `fallback(ctx)`, so they keep working), position it to fill
the cover, and add a class such as `has-art` once it loads so the recipe stays as the loading / missing-art state.
Then review each design's `.cover::before / ::after` rules for `has-art`: starlight draws its scene and grain there,
and petal's `.pv-flip` mirrors the whole `.cover` (limit it to covers without art).

## 9. Motion

- Each design has one orchestrated moment on opening Now Playing (base: device power-on with a scanline sweep; lull:
  windows boot in a stagger with one glint; starlight: the card stack deals in and a shooting star crosses the art;
  glasshouse: a light sweep crosses the LCD as the track slides in).
- Functional motion only elsewhere: glasshouse's cards fade-rise 8 px on route change, press sinks, sheets slide, popovers fade-rise 4 px, segment indicators slide, the
  transfer hatch drifts only while bytes arrive, waveform bars breathe only while playing, sparkles twinkle (max 5 per
  view), lyrics scroll the current line into place, a 240 ms shake when seeking past what is downloaded.
- Only `transform` and `opacity` animate; keyframes only for loops; loops pause under `:root.is-hidden`.
- **Reduced motion** (OS setting, or Settings > Appearance > Motion, which sets `:root[data-motion="reduce"]`;
  `data-motion="full"` overrides the OS): base.css stops every animation and limits transitions to colour / opacity
  (120 ms), so anything a design adds obeys it automatically. JS checks `App.reducedMotion()` (scroll behaviour, track
  change, lyrics). Classic's CRT entrance becomes a fade.

## 10. Accessibility

- Contrast: text 4.5:1, large text and UI parts (edges, icons, focus, seek layers) 3:1, on every surface, in both
  themes, in every design. Pairs on gradients and glass are checked on the worst stop and recorded as `check:` comments.
- Focus: `:focus-visible` draws a 2 px `--focus` outline on every control; never removed. A skip link leads to content.
- Keyboard: everything works without a mouse; shortcuts are ignored while typing and inside `.webamp` /
  `[data-webamp]`. Shortcut list (`App.shortcuts`, shown by `?`): Space play/pause; Left/Right seek 5 s (Shift 30 s);
  N / Ctrl+Right next; P / Ctrl+Left previous; Up/Down volume (outside lists); M mute; S shuffle; R repeat; L like;
  Shift+S save; Q queue; Y lyrics; W Classic; T theme; Esc close; `/` search; Ctrl+K palette; G then H/S/L/P go to
  Home/Search/Library/Playlists; Ctrl+Shift+M mini player (desktop app). Lists: Up/Down move, Enter play, Shift+Enter
  play next, Alt+Up/Down reorder, Delete remove.
- Every drag has a non-drag equivalent (menus, Alt+arrows, "Add to playlist").
- Names and live regions: icon-only buttons have `aria-label` + tooltip; decorative art is `aria-hidden`; covers get a
  label only where they are the content; `App.announce` reports async results (search done, saved, health checked);
  tablists use roles and arrow keys; sheets and dialogs trap focus and return it; the Classic stage hides the app behind it (`inert`).
- No colour-only state: status, source, quality and save states each have a glyph shape and a text label.
- No layout shift on state changes: buffering, saving and errors swap content inside fixed boxes.
- Text size pref 90 / 100 / 112 / 125 % scales the root; spacing is in rem. Hit targets >= 44 px (`--hit`).

## 11. Responsive

| Width | Layout |
|---|---|
| Phone < 640 | header (brand, compact tray, Activity, theme), content with 16 px gutters, mini player above a 4-tab bar (Home, Search, Library, More; More holds Playlists, Liked, History, Following, Stats, Settings, Classic, Activity, Shortcuts). Now Playing is a full sheet with Art / Lyrics / Queue / Details tabs (compact hero on the last three). Row actions live behind each row's more button. Safe-area insets respected. Two or more banners collapse to the most severe plus "more notices". |
| Tablet 640-1099 | `rail` shell: 72 px icon rail; `topnav`: icon-only pills and a compact tray; `dock`: floating icon dock under the player bar. Context panel = right overlay drawer. |
| Desktop >= 1100 | the design's shell; context panel overlay from 1100, docked from 1360. Player bar full width. |
| Short landscape (height < 600, width < 1100) | compact Now Playing hero; the tab bar hides while Now Playing is open. |

Track lists use a container query (`.tlist`), so they adapt to their column, not the viewport. Shelves scroll-snap
(168 px tiles, 148 px on phones). Covers: `App.ui.cover` writes `--cover-size-base` inline; set `--cover-size` in CSS to
resize one in a given place.

## 12. Classic mode (Webamp)

The full integration recipe is in the header comment of
[prototype/screens/classic.js](prototype/screens/classic.js); the essentials:

- **Classic is only Webamp.** In a browser the stage is a full-viewport layer (`--z-classic`, under sheets, popovers
  and toasts opened from it) holding the design's wallpaper, Webamp and one quiet way back: a small "Back to lull"
  button in the corner (Esc, W and Webamp's close button leave too). In the desktop app Webamp has its own borderless
  window (docs/context/desktop.md). The prototype's title pill (Exit / Skins / Help), skin drawer and footer notice
  are gone: what they held lives in Settings > Appearance > Classic. Webamp renders into `#webamp-stage` (marked
  `data-webamp`, `position: relative`) with `webamp.renderInto(el)` (Webamp >= 2.3.0).
- **Version and vetting**: npm `webamp`, MIT. Designed against 2.3.1. Pin the exact version with a lockfile, bundle it
  (no CDN), lazy-load it the first time the stage opens (never on boot), read the diff before any upgrade.
  `webamp/butterchurn` (Milkdrop) is optional and much larger.
- **Options**: `enableHotkeys: false` (the app owns shortcuts), `enableMediaSession: false` (the app owns Media
  Session and forwards actions while Classic is open), `zIndex` just above `--z-classic`, `handleTrackDropEvent` into
  the app queue, `handleSaveListEvent` / `handleLoadListEvent` to M3U export / import, `availableSkins` from favourites,
  always pass `metaData` and `duration` per track (otherwise Webamp fetches each file's head).
- **Hand-off** (the design assumes it): on enter, pause the app engine and give Webamp the queue, index and position;
  Webamp has no play / pause / seek events, so read `getMediaStatus()`, `getPlaylistTracks()`, shuffle and repeat back
  on exit. Webamp's own close button exits Classic (`onWillClose(cancel)`). Create the instance once; `close()` /
  `reopen()`. Settings says what differs: crossfade, loudness normalisation and gapless pause, repeat is on/off.
- **Skins** from the Winamp Skin Museum (public GraphQL `https://api.webamp.org/graphql`, CORS allowed):
  `search_classic_skins(query, first: 30, offset, nsfw: false)` while searching (debounce 300 ms), else
  `skins(first: 30, offset, sort: MUSEUM, filter: APPROVED)`. Use `md5` as the skin id. Screenshots are 275x348 PNGs,
  shown with `image-rendering: pixelated`. `.wsz` downloads send no CORS headers, so the server downloads, validates
  (a zip holding `main.bmp`) and caches skins and serves them same-origin. NSFW skins are hidden unless Settings >
  Appearance turns them on. Attribution is in the skin browser: "Skins from the Winamp Skin Museum" (links to
  skins.webamp.org) and "View on the Winamp Skin Museum" per skin (`museum_url`). No terms of use or rate limits were
  published when this was designed: page, debounce and cache to stay polite.
- **Settings > Appearance > Classic** (lull's own controls, every design): the skin in use with "Open a .wsz file" and
  "Use Webamp's base skin"; the skin browser (Browse / Favourites / Downloaded tabs, search, screenshot cards with Use,
  favourite star and museum link; a screenshot opens larger in a popover); Show NSFW skins; what differs in Classic,
  with a way in. The choice is per device and reaches an open Classic at once, in other tabs and in the app's window.
  A `.wsz` opened there or dropped anywhere on lull is kept on the device and chosen.
- **States**: in Settings, downloading (through the hub) then in use, or couldn't download with the hub's sentence and
  Try again; offline (downloaded skins only). In Classic, loading the skin, and failed (the previous skin stays; a
  load that hasn't resolved after ~10 s counts as failed; a link to Settings). Webamp's own `alert()`s never reach the
  browser: they become these sentences or toasts. On phones Webamp runs at 1x with the equalizer closed (275 px fits a
  360 px viewport).

## 13. Media Session and PWA

- **Media Session** (app.js section 13): metadata (title, artist, album, artwork 96 / 256 / 512 PNG from the server's
  origin), `playbackState`, handlers `play pause stop previoustrack nexttrack seekto seekbackward seekforward`
  (default offset 10 s), and `setPositionState` on every track change and seek. `seekto` clamps to what is buffered,
  like the seek bar. Without cover art use [assets/fallback-art-96.png](assets/fallback-art-96.png) / `-256` / `-512`.
  The prototype skips artwork on `file://`. The desktop wrapper relies on this path for OS media keys.
- **PWA**: no manifest ships; use `name`/`short_name` "lull", `display: standalone`, icons
  [assets/icons/icon-192.png](assets/icons/icon-192.png), `icon-512.png` and `icon-maskable-512.png`
  (`purpose: maskable`), `background_color` / `theme_color` = the default design's `--sky` (lull day: `#f7f4ff`;
  night: `#2a2c5a`). app.js already updates `<meta name="theme-color">` from `--sky` on every theme or design
  change. Install button (captures `beforeinstallprompt`), iOS instructions sheet and the update toast are designed
  (`system-settings-app`, `system-ios-sheet`, `system-update-toast`).
- App icon masters: [assets/app-icon.svg](assets/app-icon.svg), [assets/app-icon-maskable.svg](assets/app-icon-maskable.svg);
  every packaged size is in [assets/icons/](assets/icons/) (table in [desktop/DESKTOP.md](desktop/DESKTOP.md) section 8).

## 14. Do-nots

1. No hard-coded colours in components or screens: semantic tokens only. Designs set tokens and restyle under their
   own `:root[data-design]` scope.
2. No colour-only state; no pastel text on pastel; nothing below the contrast floors in either theme.
3. No CDN, no runtime font or icon downloads: everything self-hosted (OFL fonts, at most two extra families per
   design).
4. No moodboard artwork, logos or third-party illustrations in the UI; covers and ornaments are drawn.
5. No dead buttons, no "coming soon"; every control maps to an action, a hook, a toast or a state.
6. No animation of layout properties; nothing that ignores reduced motion; no loops while the document is hidden.
7. No blur without a solid fallback that passes contrast on its own (glasshouse is the only design that uses it).
8. No gloss on surfaces: gloss is for the few things you press (play, device keys, primary buttons per design) and
   window chrome.
9. Sparkles: at most 5 per view, in empty space, never on text.
10. Never reveal or edit config secrets in the UI (tokens, API keys, credentials, URLs are read-only with "set in
    server config").
11. Don't let Webamp own hotkeys or Media Session; don't load it before Classic opens.
12. Don't remove the hidden hooks (`.card-head`, `.region-title`, `data-now`, `data-dk`, `data-region`): designs depend
    on them even though the base look hides them.
