# lull: UI design package

A complete UI design for the self-hosted music player in your brief (Soulseek streaming, Jellyfin saving, discovery,
Last.fm, a Webamp "Classic" mode, PWA, Tauri wrapper for Windows and Linux):

- **Five switchable designs** on one shared design system: `lull` (default), `skydeck`, `starlight`, `petal`,
  `glasshouse`. Each has a day and a night theme; switch in Settings > Appearance, no reload.
- **A clickable prototype** of every screen and state (about 170 states: streaming, stalled peers, save pipeline failures,
  lyrics variants, Classic mode, banners, empty states...), driven by fictional mock data.
- **The design system** as real, reusable code: tokens, base styles, components, icons, self-hosted fonts.
- **Desktop wrapper designs** for Windows and Linux: title bars, mini player, tray menu, first-run connect screen,
  close-to-tray dialog, and the decisions behind them.
- **App icons** in every size you need (Windows `.ico`, Linux hicolor, PWA, tray), plus Media Session fallback art.

## How to view

- **Prototype:** double-click [prototype/index.html](prototype/index.html). It opens straight from disk in Chrome, Edge
  or Firefox: no server, no build, no network.
- **Desktop wrapper board:** [desktop/preview.html](desktop/preview.html) (windows per OS, mini player, tray, connect
  screens, icons).
- **Flags** go after the route in the address bar: `#/home?design=petal&theme=night`. `design` = `lull skydeck
  starlight petal glasshouse`; `theme` = `day night system`; `motion=reduce` previews reduced motion.
- **States:** the "states" button (bottom right) lists every screen state and opens it. It exists only in the
  prototype; `switcher=0` hides it.

## Gallery

| Design | Day | Night | Now Playing | Phone |
|---|---|---|---|---|
| **lull**: glossy lavender Y2K windows, pill nav, orb title bars | [![lull home day](screenshots/lull-home-day.webp)](screenshots/lull-home-day.webp) | [![lull home night](screenshots/lull-home-night.webp)](screenshots/lull-home-night.webp) | [![lull now day](screenshots/lull-now-day.webp)](screenshots/lull-now-day.webp) | [![lull phone](screenshots/lull-phone.webp)](screenshots/lull-phone.webp) |
| **skydeck**: a pocket handheld under a dusk sky, cream trays | [![skydeck home day](screenshots/skydeck-home-day.webp)](screenshots/skydeck-home-day.webp) | [![skydeck home night](screenshots/skydeck-home-night.webp)](screenshots/skydeck-home-night.webp) | [![skydeck now night](screenshots/skydeck-now-night.webp)](screenshots/skydeck-now-night.webp) | [![skydeck phone](screenshots/skydeck-phone.webp)](screenshots/skydeck-phone.webp) |
| **starlight**: a starry-night poster, stacked cream trays, burgundy caps | [![starlight home day](screenshots/starlight-home-day.webp)](screenshots/starlight-home-day.webp) | [![starlight home night](screenshots/starlight-home-night.webp)](screenshots/starlight-home-night.webp) | [![starlight now night](screenshots/starlight-now-night.webp)](screenshots/starlight-now-night.webp) | [![starlight phone](screenshots/starlight-phone.webp)](screenshots/starlight-phone.webp) |
| **petal**: pink pixel playlist pages, a glossy pink player window | [![petal home day](screenshots/petal-home-day.webp)](screenshots/petal-home-day.webp) | [![petal home night](screenshots/petal-home-night.webp)](screenshots/petal-home-night.webp) | [![petal now day](screenshots/petal-now-day.webp)](screenshots/petal-now-day.webp) | [![petal phone](screenshots/petal-phone.webp)](screenshots/petal-phone.webp) |
| **glasshouse**: frosted silver media windows on a misty midnight desktop | [![glasshouse home day](screenshots/glasshouse-home-day.webp)](screenshots/glasshouse-home-day.webp) | [![glasshouse home night](screenshots/glasshouse-home-night.webp)](screenshots/glasshouse-home-night.webp) | [![glasshouse now night](screenshots/glasshouse-now-night.webp)](screenshots/glasshouse-now-night.webp) | [![glasshouse phone](screenshots/glasshouse-phone.webp)](screenshots/glasshouse-phone.webp) |

Desktop shots are 1440x900 scaled to 1280 wide; phone shots are 390 wide.

## What's where

| Path | What it is |
|---|---|
| [DESIGN.md](DESIGN.md) | The spec: principles, the five designs, architecture, tokens, components, every screen and state, the prototype API and exactly what to replace with server calls, motion, accessibility, responsive rules, Webamp, Media Session, do-nots |
| [desktop/DESKTOP.md](desktop/DESKTOP.md) | The Tauri wrapper spec: windows and title bars per OS, mini player, tray, media keys, connect screen, icons |
| `tokens.css`, `base.css`, `components.css` | Design system: tokens, surfaces, type, focus, reduced motion, every component and the app shell |
| `icons.js` | Icon sprite and the `icon()` helper |
| `fonts/` | Shared fonts (Nunito, Jersey 10) with licences |
| `designs/` | `designs.js` (registry and switching) and one folder per design (`design.css`, optional `design.js`, fonts, art) |
| `prototype/` | `index.html` (shell and load order), `app.js` (core), `mock-data.js` (fictional data), `screens/` (every screen) |
| `desktop/` | Wrapper UI (`desktop.css`, `desktop.js`), mini-player window (`window.html`), design board (`preview.*`) |
| `assets/` | Wordmarks, app icon masters, Media Session fallback art; `assets/icons/` holds every packaged icon size; `assets/previews/` the design picker screenshots |
| `screenshots/` | The gallery above |
| `inspo/` | Your moodboard, which the designs were drawn from |

## Hand this to your agent

Unzip this package as `design/` at the root of your repo, then paste:

> The UI design for this app is in `design/`. Read `design/DESIGN.md` first, then `design/desktop/DESKTOP.md` for the
> Tauri wrapper. Build the real web UI from these files: reuse `tokens.css`, `base.css`, `components.css`,
> `designs/`, `icons.js`, `fonts/` and `assets/` as they are; port the screens in `design/prototype/` to real data,
> keeping their markup, classes and behaviour; replace the mock data and the mock player and save engines with calls
> to our server API (DESIGN.md section 8 maps every mock to the endpoint it needs); remove the prototype-only parts
> (everything marked `PROTO`, the states switcher); keep the design switcher and all five designs. Build the Tauri
> wrapper as `design/desktop/DESKTOP.md` describes. `design/inspo/` is the moodboard, for reference only. Open
> `design/prototype/index.html` to see any screen or state while you work.

## Notes

- **The name.** "lull" and its wordmark are suggestions. The name lives in one place (`Designs.product` in
  `designs/designs.js`), plus the wordmark and icon artwork in `assets/`; search for a trademark clash before a public
  release.
- **Data.** Every artist, album, track, peer and listener name in the prototype is made up.
- **Licences.** Fonts are under the SIL Open Font License 1.1 (licence files next to each font; the starlight and lull
  wordmarks are outlined from OFL fonts, licences included). Everything else in this package was made for your project
  and is free to use, change and ship in it. Webamp and Winamp skins are not included: the app loads them at runtime
  (see DESIGN.md section 12). The `inspo/` images are your moodboard and are not part of the UI.
