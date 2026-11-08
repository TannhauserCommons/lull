# ADR-0101: One live model for themes, any value anywhere, markup in every region

- Status: accepted (the owner approved it on 2026-11-08)
- Date: 2026-11-08

## Context

Three complete listener themes were built against the theme contract (docs/themes.md) from approved mockups, each a
world of its own. All three pass the gates. All three also drifted from
their mockups, and about a quarter of each `theme.css` (101 to 180 rules) hides, moves, re-orders, fixes in place or
re-labels lull's own parts rather than expressing the theme's idea. Where the contract made the mockup hard, the
builders fell back to lull's usual look, because that was the part that came already wired.

What they fought, from their code and their own notes:
- an LED over the page that names the song and its state: lull's phone mini player (`#mini`), hidden above 640 px,
  switched on at desktop sizes and fixed at the top, its keys hidden, because "a theme can't copy text into new places".
  The builder looked for the title in the like button's `aria-label` first;
- a pocket player's LCD in the player bar showing source and quality: the bar's chips moved into it with CSS anchor
  positioning (Chromium only), drawn under the LCD, then lifted with `z-index`;
- a glass column holding lyrics, "up next" and the queue: lull's cards `position: fixed` into it at hand-measured heights
  (206 and 290 px), with the header clipped to a fixed height so the sums hold;
- the time of day on every mockup ("14:12"): lull gives themes no clock, so one theme shows the weekday, sized by
  hand-measured widths per English weekday name;
- "next ▸" on two of the mockups: the next song exists only as a field of Home's lead slot;
- Like and Save among the player's tools: placed `position: absolute` at `right: calc(12px + var(--fit-side, 336px) -
  88px)`, outside the fit, so they never give way, and a padding reserved to feed the fit's measure;
- the repeat mode as a word ("RPT 1"): read with `[aria-label*="one" i]`, which breaks with any rewording;
- words on controls (SHUF, REC, 保存): each hides lull's icon and writes `content:` on a pseudo-element lull already uses
  for the pressed state, then restores that state;
- a save key and the song's source in the desktop mini player: no `save` key exists, and there is no source field;
- the mini player's song area: lull replaces whatever is inside `data-dk-now` on every song (`desktop.js` `paintTrack`),
  and the docs said to keep `data-dk-now`, so a theme copied lull's internal markup into it to get a first paint that
  matches, then hid the cover lull puts there;
- a title sized to its length, a barcode drawn from the song, a region's height: none of them reach CSS;
- the state's kanji, written three times (`#mini`, the mini window, the strip), because the mini windows don't share
  the page's root states;
- previewing the mini player: done by faking the desktop app's bridge in a script of their own; `themes:check` always
  draws all five built-ins at one desktop size, and nothing checks text against the picture actually behind it, where
  white type on pale cloud measured 1.8:1 while looking fine because of its shadow.

Some of it was not the contract: `.disc-mod[data-module]` already names Home's modules (they used `:has()`), a plain
`data-act` button in a slot already works, and `!important` in a theme does nothing (the `user` layer already wins, and
it can never beat lull's own `!important`). The docs didn't make those easy to find; they are fixed with this.

The owner's ask goes past these three themes: more freedom in CSS, and more that moves with what's happening, in a
modular way, so that a theme that wants to do something interesting or show another value doesn't wait for a hook
to be patched in for it.

### The causes underneath

Reading lull's code behind each of these, they come down to four causes in the contract and one in the tools.

1. **Values are wired one by one, to places, not published.** A slot's `{{field}}` is filled once, when the slot is
   drawn, and is stale after that: Home's lead keeps the song Home was drawn with (nothing in `discover.js` draws it
   again when the song changes), and so does the greeting's time of day. What stays live is wired three separate ways,
   each to one place: `data-bind` (the song's times and the transport's pressed states), `data-now` (Now Playing only,
   refreshed by `now.js`), and `data-dk-*` (the mini player windows only). `--status-text` is written on `#pbar` and
   `#mini`, `--song-pos` on six named elements, `data-player` on `:root`, each by its own code. Each slot has its own
   field list. So every value a theme wants somewhere new is a code change in lull, and some values exist nowhere: the
   clock, the next song outside one slot, the source as a word, the repeat mode as a state.
2. **A theme can add markup in six places, with a different set of live parts in each.** When the idea needs a part
   where lull has none (an LED over the header, an LCD in the bar), the only way to make it is to take one of lull's
   parts away from its job, or teleport it with fixed or anchor positioning. The live parts (a play key, the seek, the
   cover) are offered per slot, so what one slot can hold another can't.
3. **lull redraws the hosts it owns with its own markup.** `paintTrack` rewrites `data-dk-now` and the strip's cover;
   `now.js` swaps whole `data-now` hosts. A theme that put its markup there loses it at the next song, so it copies
   lull's internal markup instead. Live controls came "for free" only inside lull's ready-made parts, which is why the
   first build restyled lull's handheld device as the theme's player instead of building the approved one.
4. **Layout policy lives in lull's markup and script, not in CSS.** The order in which parts give way is a `data-fit`
   number in lull's markup; the player bar's mirrored sides are computed for lull's three columns. A theme that wants
   other priorities (Like before the tools) or another shape can't say so, so it steps outside the fit with absolute
   positioning and magic numbers, or patches width bands.
5. **The authoring tools were built for the five built-in designs.**

lull's own `!important` rules are not one of the causes: there are 30, and nearly all hold something that must hold
whatever a design draws (a part that gave way leaves the row, a strip hides the panel's contents, reduced motion stops
movement). They are kept to that and documented.

## Decision

**lull provides the features, the values and the actions; the theme decides everything about layout and look; the
gates guarantee that every feature stays reachable and accessible.** Nothing lull's script decides about where a part
sits or what shape a region takes is fixed: each such choice is a default the theme can replace from CSS. What lull
holds fixed is what makes it work for everyone: every feature reachable, every control with its name, keyboard and
screen-reader order, readable text, and the cost on a slow device.

Six mechanisms carry it, each general, so a theme that wants something new uses them rather than waiting for a hook.
Each is additive: theme `api` stays 1 (docs/themes.md section 9), every default stays as it is, and no built-in design
or listener's theme changes how it looks or works.

### 1. One live model, published the same way for every value (cause 1)

`web/designs/live.json` declares lull's live state as a tree of named values. Each value has a path (`song.title`), a type
(`text`, `number`, `state` with its named states, or `flag`), and how often it can change (`song`, `press`, `minute`,
`second`, `tick`). One module, `web/designs/live.js`, computes the model from what the app already holds and publishes every
value by one rule, so adding a value is one entry in `live.json` and one line that reads it, and every form below,
every window, the hub's check, Theme Studio and the docs' table follow by themselves:

- **As text, anywhere:** `<b data-bind="next.title"></b>` holds the value's words, kept current in place, in any slot,
  in lull's page and in the desktop mini player windows alike.
- **As a custom property, on live hosts:** `song.title` is `--song-title` (a CSS string, for `content:`), `clock.minute`
  is `--clock-minute` (a number, for `calc()`, `rotate` and `counter()`), on every live host: lull's own (`#pbar`,
  `#mini`, the mini windows' `[data-dk-player]`, Now Playing's hero, Home's lead, `.sky-specks`), every
  `[data-region]`, and any element a theme marks `data-live`. Registered `inherits: false`, as `--song-pos` is, so a
  change restyles the host alone; a part inside draws with it through `inherit`.
- **As a state, on `:root`:** a `state` or `flag` is also an attribute on the root of every window (the page and the
  mini windows), `data-song-source="soulseek"`, `data-player-repeat="one"`, `data-clock-daypart="evening"`, so one rule
  follows it anywhere.

The first model (the exact list is `web/designs/live.json`; adding to it keeps `api` 1):

| Branch | Values |
|---|---|
| `clock` | `time` (the device's own format), `hour`, `minute`, `second`, `weekday`, `date`, `month`, `year`, `daypart` (`morning` `afternoon` `evening` `night`) |
| `song` | `title`, `artist`, `album`, `year`, `from` (the place it plays from), `source` (`library` `device` `soulseek` `youtube`), `quality`, `format`, `lossless`, `liked`, `saved`, `pos`, `elapsed`, `duration`, `buffered`, `received`, `hue` and `seed` (stable per song, for decoration), `title-length` |
| `next`, `queue.1` ... `queue.8` | `title`, `artist`, `album`, `source` of the songs after it; `queue.count`, `queue.left` |
| `player` | `status`, `status-text`, `playing`, `shuffle`, `repeat`, `volume`, `muted`, `crossfading`, `private` |
| `app` | `screen`, `route`, `view`, `ctx`, `size`, `height` (as on `.app`, now also on `:root`, for the wallpaper behind it) |
| `services` | `soulseek`, `library`, `lastfm` (`ok` `warn` `down`) |
| `week` | `time`, `plays`, `artists`, `peak` (the 7-day stats Home's lead shows today) |
| `region` | each region host's own `--region-width` and `--region-height`, written when it resizes |

**A theme pays only for what it uses.** The hub reads which of the model's values a theme names (a `data-bind` or a
`{{field}}` in its slots, a `var(--song-title)` or a `[data-clock-daypart]` in its CSS) when it reads the package, and
lists them in the manifest it serves, as it fills in `slots` today; the built-in designs' package scripts carry the
same list, written by the same code. The page loads `web/designs/live.js` only for a design or theme whose list isn't empty,
and publishes only the values on it. A value nobody names is never computed or written, and the clock's timer runs only
when something names a `clock` value, once a second only when something names `clock.second`. So a design that uses
none of the model costs what it costs today, its download included, and one that uses a little pays for that little.
A custom property a theme sets itself (`--clock-hour: 3`) is the theme's own: lull doesn't write one the theme
declares, so a name that happens to match never changes an existing theme.

The names lull uses today stay, as they are (`--song-pos`, `--status-text`, `data-player`, `data-playing`, the slot
fields, `data-now`, `data-dk-*`): they are the same values under their old names. Slot fields gain the whole model by
path (`{{clock.time}}`), as drawn; a binding is the value now.

The hub refuses a slot that binds a path `web/designs/live.json` doesn't list, naming the branches it has, as it does for the
mini player's keys today: a theme never ships a value that stays empty.

### 2. Markup and live parts in every region (cause 2)

- **A slot at each region's edges:** `slots/before.<region>.html` and `slots/after.<region>.html` for each shell region
  (`rail`, `header`, `content`, `ctx`, `nowcol`, `playerbar`, `mini`, `tabbar`), drawn as the region's first or last
  child in a `.region-slot[data-slot]` box the theme places with its CSS. Keyboard and screen-reader order is the
  markup's, so a slot `before` a region is read first. lull draws a region slot when the shell is drawn and never again
  for a song or a state: values reach it by the model. A theme with none adds nothing to the page.
- **A slot at each screen's edges:** `slots/before.<screen>.html` and `slots/after.<screen>.html` (`home`, `now`,
  `search`, `library`, `album`, `artist`, `playlist`, `stats`, `settings`...), drawn inside the screen's own grid
  (`.disc-home`, `.now__grid`...), so the theme's boxes and lull's parts are items of one grid: a window drawn behind
  the greeting and the first module is a box of the theme's in the same grid area, not a pseudo-element stretched
  behind two of lull's boxes.
- **One catalogue of live parts, in every slot:** every `<lull-part>` works in every slot, instead of a list per slot:
  the transport keys, like, save, the seek, the volume, the cover, the source badge, the quality chip, playing-from,
  the facts, the state, up next, the lyrics. Each is lull's own markup, wired and kept current by lull. With plain
  `data-act` buttons and `data-bind` words a theme needs no part at all; the parts are there when it wants lull's.
- **So a part goes where the idea puts it, without being teleported:** a column of lyrics and up next beside the page
  is the theme's markup in `after.ctx` (or `before.content`) holding the `lyrics` and `up-next` parts, with lull's own
  cards hidden on that screen; nothing is fixed in place at measured heights, and each region's size is in the model
  for what still needs it.

docs/themes.md gains a section on building from a mockup: each part of the mockup is either the theme's markup (a
slot, the model's values, `data-act` buttons) or one of lull's parts placed by grid, lull's own version hidden where
the theme draws its own (a heading kept for screen readers), and which of lull's screens are already one grid.

### 3. lull never rewrites a theme's markup (cause 3)

Everything lull keeps current inside a theme's markup, it changes in place: a binding's text node, a part's own
element, an attribute, a custom property. lull's own blocks inside slots (`data-dk-now`, the `data-now` hosts, the
`device` and `compact` parts) stay lull's default pieces, which a theme may use or leave out; `data-dk-now` becomes
optional. The desktop mini player gets the model and the key `save`. Home's lead is kept current by its bindings like
any other slot. A built-in design's lead, drawn from fields, stays as it is today: drawn with Home.

### 4. Layout belongs to the theme: every layout choice is a switch CSS sets (cause 4)

Each choice about layout that lull's script makes today (whether the side panel docks beside the page or opens over
it, which way its handle drags, when the Now column opens or folds to a strip, which width class the main area is in,
what gives way first in a row that runs out of room, whether the player bar mirrors its sides, and every other one the
audit for this slice finds in `app.js`, `layout.js` and the screens) becomes a named **layout switch**: a custom
property on `.app`, `--layout-<switch>`, that lull's script reads instead of deciding. lull's base CSS sets each switch
to what it does today, with the same widths, so nothing changes until a theme sets one. A theme sets any of them with
any rule it likes (a media query, a container query, a screen, a state): a queue along the bottom that docks at every
width and drags by its height, a Now column that never folds, a bar that gives way in its own order. lull reads the
switches in the pass where it already lays out (a resize, a design switch, a fit), one computed style each.

Parts that give way keep `data-fit` as their default order, and `--fit-order` replaces it. `display: contents` on a
wrapper hands its parts to the grid around it, which the fit already follows; the docs show it.

A new layout choice in lull's script is a switch from the start: a check in `mise run check` fails a width, a height or
a breakpoint compared in `web/` script outside the switches' reader. lull's own `!important` rules stay limited to what
must hold whatever a design draws (a part that gave way leaves its row, a strip hides its panel's contents, reduced
motion stops movement), each with its reason, listed in docs/themes.md, and the same check keeps new ones to that list.

### 5. Authoring tools for listeners' themes (cause 5)

`mise run themes:check` gains `--only <ids>` (and `--mine` for every theme in `LULL_THEMES_DIR` and none of the
built-ins), `--sizes` (several desktop sizes in one run), a `mini` route that draws the desktop mini player's device
and strip in a plain browser through the stand-in bridge the gates already use, and a contrast check against what is
actually drawn behind each line of text (the picture under it, not the token it was meant to sit on), naming the worst
spot and its ratio, and `--mockup <file.html>`, which draws the mockup beside the theme at the same size on every
screen it covers, so drift shows while the theme is built rather than at the end. Theme Studio lists the model's values
from `web/designs/live.json`.

### 6. The gates guarantee features, not layout (what makes 1 to 5 safe)

A list of lull's features, `web/designs/features.json` (play, previous, next, seek, volume, like, save, shuffle,
repeat, the queue, lyrics, activity, search, Home, the library, playlists, settings, Now Playing, the player's "…",
playing on another device, the equaliser...), each with the action that reaches it. A gate opens every screen at every
width in each built-in design, and `themes:check` in a listener's theme, and fails a feature that no visible, clickable,
named control reaches, either on the screen or in a menu or dock one click opens. With it, a theme may hide, move,
group, fold or redraw any of lull's controls, and the gate still knows every feature is there. The existing function
checks stay: every control clickable, every text readable, at every width.

The actions a theme's own buttons use (`data-act`) are listed and documented the same way, so a theme's dock, LCD or
remote reaches every feature lull's own controls do.

### What stays lull's

- Every feature reachable, every control named, text readable (the gates above).
- Keyboard and screen-reader order is the markup's: lull's, with a slot's markup where the slot sits. CSS moves what is
  seen, not what is read.
- A part of lull's page stays in its region: a theme shows it elsewhere by placing the part there (section 2) and
  hiding lull's.
- Long lists scroll in their own scroller, which lull's long-list drawing needs.
- Themes run no script (ADR-0004).

## Constraints

- **Additive.** `api` stays 1. Every new name is new; no existing token, hook, field, part, slot or key changes meaning.
- **Defaults unchanged.** The built-in designs ship no region slots and bind nothing new, so their pages gain no
  elements; their goldens don't move.
- **Nothing a design or theme does today changes.** A slot that keeps `data-dk-now` still gets lull's block; a part's
  markup `data-fit` still decides its order unless a theme sets `--fit-order`.
- **Performance on low-end devices holds** (the perf gates, `tests/ui/gates/perf.spec.mjs`, CPU slowed 4×):
  - nothing is published that the loaded design or theme doesn't name, so the built-ins do no new work;
  - nothing updates per frame. A value is published only when it changes, at its own pace: once a song, once a press
    or a state, once a minute; the song's place a hundredth at a time, as now;
  - the clock is one timer set to the next minute's edge, as the easter eggs' clock is. `clock.second` and
    `song.elapsed` tick once a second only for a theme that names them;
  - work pauses while hidden: the clock's timer stops while the page is hidden and catches up once when it shows;
  - no layout thrash: writes skip unchanged values, lookups use the cached `$$kept`, region sizes come from a
    `ResizeObserver` (written on resize, never read in a loop), and the fit reads its new properties in the pass where
    it already reads each part's style;
  - a custom property written on a live host restyles that host alone (`inherits: false`), never a page; a root state
    restyles only what a rule selects with it.

## What it costs

Only for a design or theme that names a value; for one that names none, nothing here runs.

| Mechanism | Work | When |
|---|---|---|
| Reading what a theme names | one pass over its CSS and slots, on the hub | when the hub reads the package |
| `web/designs/live.js` | one more script to download | only for a design or theme that names a value |
| Clock | one timer; the named minute values written on the live hosts and `:root` | once a minute (once a second only where asked); none while hidden |
| Song, next, player | the named values written where they changed: bound text, live hosts, `:root` | once a song, a press or a state; `song.pos` a hundredth at a time, as today |
| Parts in slots | that part's own markup | as lull draws that part today |
| Region slots | one template each | when the shell is drawn (a design or theme switch) |
| Screen slots | one template each | when that screen is drawn, as the screen itself is |
| Region sizes | two properties per region | when a region resizes |
| Layout switches | one computed style read per switch | where lull lays out today (a resize, a design switch, a fit) |
| Tools | none in the app | in `themes:check` only |

Measured with the perf gates before each slice lands: the built-ins' budgets stay where they are, and a new scenario
draws a test theme that binds every value, in every region, and holds it to the same budgets (`tickRestyleElements`,
`idleFrames`, `playingMsPerSecond`, `fanOutElements`). A unit test holds every value in `web/designs/live.json` to all its
forms (binding, property, root state) and to the docs' table.

## Order of work

Vertical slices, each through the queue with its docs, gates and tests:
1. the model's frame (`web/designs/live.json`, `web/designs/live.js`, bindings, live hosts, root states, the hub's check) with its first
   branch, the clock;
2. the `song`, `next`/`queue`, `player`, `app`, `services`, `week` and `region` branches, in the page and in the mini
   player windows, the mini `save` key, and Home's lead kept current;
3. region and screen slots, the one catalogue of parts, and the docs' section on building from a mockup;
4. layout switches: the audit of every layout choice in lull's script, each made a switch, and the check that keeps it so;
5. `themes:check` for listeners' themes, the mini player preview, the rendered-contrast check and the mockup beside
   the theme;
6. the features list, the actions list and the gate that every feature stays reachable.

## Alternatives

- **One hook per gap** (a `next` field in `nowHero`, a `save` key, a clock in the LED slot...). Each closes one
  complaint and leaves the cause: the next theme meets the same wall one place over. The three themes asked for the
  same values in five different places.
- **A fixed list of values, each wired by hand.** Better than one hook per gap, but every new value is still new code
  in several places (each window, the hub, the docs). One model published by one rule makes a new value one line.
- **Let themes run script.** Any layout and any value, at the cost of every promise a theme makes today: no code, no
  reach outside the hub, nothing a listener has to trust (ADR-0004). Not considered further.
- **Every value as an inherited custom property on `:root`.** Simplest for a theme, but an inherited value on `:root`
  restyles every element of the page each time it changes: thousands on a long playlist, every minute for a clock.
  Live hosts give the same reach at the cost of a few elements; `:root` gets only states, which restyle what selects
  them.
- **More slots in fixed places** (an LED slot, an LCD slot). Each fits one idea of where things go; a slot at each
  region's edges fits any.

## Consequences

- A theme can show any of lull's live values anywhere it draws, draw anywhere in the shell, and follow any state from
  any rule, with no hack: no borrowed parts, no anchor or fixed positioning across regions, no `:has()` to find
  state, no words read out of accessible names.
- A value lull learns later reaches every theme by one entry in `web/designs/live.json`.
- Live values have one way in, in every window. `data-now`, `data-dk-*` and the old names keep working.
- `web/designs/live.json` becomes part of the theme API: removing or renaming a value in it is a breaking change (section 9).
- The hub refuses an unknown binding, as it refuses an unknown mini player key.
- The perf gates gain a scenario with a theme that uses everything, so a later change that makes a value costly fails
  there, not on a listener's phone.
