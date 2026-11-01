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
- Optional: `description`, `preview` (true when `assets/preview-day.webp` and `-night.webp` exist, 320×200), `coverVariety` (section 5). The hub fills in `slots` itself.

**Installing one:** drop the `.lulltheme` on Settings › Appearance › Your themes, or make it in Theme Studio there. Both store it on your hub in `<data dir>/themes/<id>/`, so every device can pick it. You can also edit the files in that folder directly: open devices reload the theme within a second.

**Safe mode:** add `?theme=safe` to the address (`https://your-hub/?theme=safe`) to boot the default design, whatever this device had picked.

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

Theme Studio writes exactly this kind of block for you (between `/* tokens: written by Theme Studio */` and `/* end of tokens */`) from its colour pickers and sliders, and warns when a text colour doesn't stand out enough from its background.

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
| Sizes | `--rail`, `--rail-icons`, `--ctx`, `--bar-h`, `--mini-h`, `--tab-h`, `--hit` |

Components read semantic tokens (`--bg`, `--fg`, `--fg-2`, `--link`, `--edge`, `--focus`...) that the surfaces `.s-sky`, `.s-panel`, `.s-card` and `.s-screen` map from the ones above, so a surface's text always pairs with its own background.

## 5. Stable hooks for full CSS

Go beyond tokens with any selector. These hooks are the theme API: they stay put across lull versions (section 9).

| Hook | Values |
|---|---|
| `:root[data-theme]` | `day`, `night` (always explicit; "match system" is resolved for you) |
| `:root[data-shell]` | `rail`, `topnav`, `dock` |
| `:root[data-motion]` | `reduce`, `full` (absent: follow the system). Keep motion off under `reduce`. |
| `:root[data-design]` | the built-in design that is loaded (your `extends`), else your id |
| `[data-region]` | `rail`, `header`, `topnav`, `content`, `ctx` (queue, lyrics, activity), `playerbar`, `mini` (phone player), `tabbar`, `lead` (Home's homeLead slot) |
| `.app[data-screen]` | `home`, `search`, `now`, `library`, `liked`, `history`, `following`, `album`, `artist`, `tag`, `playlists`, `playlist`, `stats`, `settings`, `classic`... |
| `.app[data-route]`, `.app[data-view]`, `.app[data-ctx]` | nav key; `page` or `now`; `docked`, `open`, `closed` |
| `[data-act]`, `[data-nav]` | every control's action (`play`, `next`, `like`, `shuffle`...) and nav target (`home`, `library`...) |
| `#pbar` / `#mini` `[data-status]`, `[data-playing]` | player state (`idle`, `streaming`, `buffering`, `failed`...) |
| Classes | the BEM blocks: `.card` (`.card-head`, `.card-head__title`), `.btn` (`--primary`, `--quiet`, `--sm`), `.btn-icon`, `.btn-gloss`, `.cover` (`.c1`-`.c8` recipes, `.has-art`), `.trow`, `.tile`, `.chip`, `.pill-filter`, `.seg`, `.input`, `.seek`, `.wave`, `.device`, `.empty`, `.sheet`, `.dialog`, `.toast`, `.banner`, `.nav-item`, `.nav-pill`, `.tab`, `.region-title` (hidden window titles you can show) |

`design/DESIGN.md` section 3.4 lists more state hooks (tray pills, track row file facts). Pseudo-elements are free for ornaments; keep them decorative.

**Cover variety:** with `"coverVariety": true` every cover gets stable per-item hooks, so two albums that share a generated cover still differ: `data-look="0"`-`"5"` (pick one of 2, 3 or 6 looks with `[data-look="1"], [data-look="3"], [data-look="5"]`, ...) and `--cover-flip` (1 or -1), `--cover-hue` (-28deg to 28deg), `--cover-zoom` (1 to 1.45), `--cover-dx`, `--cover-dy`.

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
| `homeLead` | `title`, `artist`, `album`, `trackId`, `format` ("FLAC :: 44.1 kHz, 1123 kbps"), `codec` ("FLAC :: 44.1 kHz"), `playing`, `liked`, `shuffle`, `repeat`, `repeatOn`, `repeatLabel`, `repeatIcon`, `likeClass`, `likeIcon`, `status`, `radio`, `date`, `month`, `weekday`, `queueCount`, `queueDuration`, `queueSize`, `next`, `week` (true once Stats has numbers), `weekTime`, `weekPlays`, `weekArtists`, `weekPeak` | `cover` (`size`), `play-button` (`size`: 40, 48, 52, 56, 64), `like-button`, `track-list` (`label`), `week-clock` (24 `<i style="--v:0..1">`, the busiest `.is-peak`), `queue-sky` (the queue as `.sky-star`s joined by a `.sky-line`) |
| `nowHero` | the `homeLead` song and player fields, `error`, `variant` | `device`, `compact` (the default hero, to pick from), `screen`, `cover` (`size`), `play-button` (`size`), `like-button` (`size`), `wave` (`bars`), `facts`, `state`, `seek`, `playing-from` |
| `miniPlayer` | `title`, `artist`, `format`, `quality` (`ll`, `ly`), `playing`, `liked`, `playLabel`, `playIcon`, `playIconClass`, `likeIcon`, `likeClass`, `opaqueClass` | `device`, `strip`, `cover` (`size`), `wave` |

Booleans render as `true` or `false`, so CSS can follow them: `[data-playing="true"]`.

Keep the live hooks when you rebuild a hero: `data-now="title|artist|album|format|screen|c-screen|c-art|led|head|state|facts|osd"` and `data-act` / `data-bind` on transport buttons (`design/DESIGN.md` 3.5 lists them all). A custom mini player keeps `data-dk-player`, the `data-dk="..."` keys and `data-dk-pos`, `data-dk-dur`, `data-dk-now`.

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
@media (prefers-reduced-motion: no-preference) { :root:not([data-motion="reduce"]) .pm-moon { animation: pm-wax 6s ease-in-out infinite alternate; } }
@keyframes pm-wax { to { box-shadow: inset -30px -6px 0 var(--link); } }
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
- a `theme.json` field lull doesn't know, or an `api` newer than this lull.

In the browser every template is also sanitized (DOMPurify), and the page's Content-Security-Policy only loads fonts, images and styles from the hub itself (`'self'`, `data:`, `blob:`).

## 9. Versioning

- `api` is the theme API version a theme is written for. lull speaks one version (now `1`) and refuses a theme that asks for a newer one, so update lull to use it.
- The API is: the tokens in section 4, the hooks in section 5, the slots, fields and parts in section 6, and the package format. Adding a token, hook, field, part or slot keeps the version.
- Renaming or removing any of them, or changing what one means, is a breaking change: it raises `api`, and the release notes say what moved. A theme written for an older `api` keeps loading; whatever it relied on that moved is ignored, never an error.
- Built-in designs' own classes (`.lull-*`, `.sd-*`, `.sl-*`, `.pt-*`, `.gh-*`) are not part of the API; extend a design for its look, but target the hooks above in your own rules.
