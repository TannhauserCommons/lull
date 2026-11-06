# ADR-0081: Sounds are packs like themes, and easter eggs are named triggers a design can answer

- Status: accepted (the owner chose: sounds on by default at 40%; while music plays only play and pause sound;
  the `still` egg breathes slowly until lull knows songs' tempo, which is separate work)
- Date: 2026-11-05

## Context

Each design is a world (ADR-0004: the five designs are themes), and a world has sounds: a gel button squishes, a
paper card clicks, a tape deck clunks. lull plays none today. The listener wants sounds that come with a look, that a
shared theme can bring along, that can be mixed and replaced one by one, and that never get in the way of the music.
They also want a few hidden surprises, rare and never in the way, that a design can dress in its own style.

What constrains it:
- **The music comes first.** lull is a music player; a click over a quiet passage is a defect, and a loop over a song
  is worse.
- **Cheap devices.** The UI gates hold every design to a frame budget, a layer budget and a 1,200-row list
  (docs/context/quality.md). A sound must cost nothing there.
- **Autoplay rules.** Browsers start audio only after a gesture, except in installed PWAs; the desktop app's
  WebView2 and WebKitGTK and Android's WebView each have their own rules.
- **Themes never run code** (ADR-0004). Whatever a theme does with a sound or a surprise, it does by declaring it.
- **Three places play:** the web UI in a browser, the desktop app (whose mini player windows send every press to
  `main`'s page, docs/context/desktop.md), and Android, where the native player plays the music and the page lives
  in a WebView.

## Decision

### The sound events

A sound belongs to an event. The names are part of the theme API (docs/themes.md section 9):

| Event | When | Default |
|---|---|---|
| `boot` | lull starts: once per app start, never on a reload | on |
| `play`, `pause` | the listener presses play or pause | on |
| `skip` | the listener moves to the next or the previous song (a song ending on its own is music into music: silent) | on |
| `seek` | the listener lets go of the seek bar | off |
| `buffering` | a loop while lull waits for sound (finding, queued, buffering), only in silence | on |
| `found` | a song lull was finding has a source | on |
| `stalled` | a transfer stalled | on |
| `saved` | a song or album finished saving to the library | on |
| `liked` | the listener likes a song (an unlike is silent) | on |
| `queued` | the listener adds to the queue (Play next, Add to queue) | on |
| `error` | a song failed, or an action the listener took failed (an error toast) | on |
| `design` | the listener switches design or theme | on |
| `classic` | Classic mode opens | on |

Easter eggs (below) may have sounds of their own. Nothing else makes a sound: no hover, focus, scroll or typing sounds, and no sound for a change another device
made.

### The pack: sounds and surprises

**One format, two homes.** A pack is a `pack.json` with the files it names: Ogg sounds in `sounds/`, and for its
surprises' looks an optional `pack.css` and `slots/egg.<name>.html`:
- **Inside a theme:** a theme package may carry `pack.json` and those files next to `theme.json`. Importing the theme
  brings its sounds and surprises; exporting it carries them. Its name and author are the theme's.
- **On its own:** a `.lullpack` file is a zip of the same, with `id`, `name`, `author` and an optional `description`
  in `pack.json`. It adds sounds, surprises or both to whatever look the listener has. It imports in Settings ›
  Sounds, on Your themes and in Theme Studio (by dropping it).

```json
{
  "id": "rain-on-glass",
  "name": "Rain on glass",
  "author": "you",
  "api": 1,
  "sounds": {
    "play": "sounds/tap.ogg",
    "skip": ["sounds/flick-1.ogg", "sounds/flick-2.ogg"],
    "buffering": { "file": "sounds/drizzle.ogg", "loop": true }
  },
  "eggs": [
    { "name": "rainy-song", "when": [{ "words": { "in": ["title"], "any": ["rain", "storm"] } }],
      "sound": "sounds/thunder-far.ogg", "once": "song" }
  ]
}
```

- A sound is a file, a list of files (lull takes the next one each time, so a repeated press doesn't sound
  mechanical), or `{ "file", "loop" }`. Only `buffering` may loop.
- An event a pack leaves out falls back to the pack under it (below); `null` silences it on purpose.
- `pack.css` is CSS with the same rules as `theme.css` (no outside URLs, its `url()`s inside the pack), loaded in the
  `user` layer after the theme's. It is meant for the pack's surprises: `:root[data-egg~="rainy-song"] ...`. It joins
  the page when the first surprise goes live, and stays: adding a stylesheet restyles the whole page, which the
  first tap must not pay for, so a rule in it that hangs on no egg may not apply until then.

**The hub validates it like a theme's assets** (`crates/themes`), with the same errors in Theme Studio, and refuses a
pack, naming the problem, for:
- a file that isn't Ogg Opus or Ogg Vorbis (read from its headers, not its name), or one nothing names;
- an event, trigger, condition or field lull doesn't know (the message lists the ones it does), or a value out of its
  range;
- **sizes:** a sound over 128 KB, a pack's sounds over 1 MB in all, more than 32 eggs;
- **lengths** (from the Ogg stream's last granule, no decoding): a UI sound over 2 s, `boot` and an egg's sound over
  6 s, a loop over 20 s;
- `pack.css` and egg slots under the theme's rules (section 8 of docs/themes.md).

Standalone packs are stored in `<data dir>/packs/<id>/`; a theme's stay in its own folder. Editing either reloads
open devices (`pack.changed`, as `theme.changed` does).

**Built-in packs** ship in `web/packs/<id>/` in exactly this format: `default` (neutral, quiet, for any look, and lull's
own surprises), and one per design, named after it: `lull` (gel-button squish, a sparkly start-up chime, a soft CRT
hum), `skydeck` (a paper card's click, a soft wheel ratchet, a radio dial tuning through static), `starlight` (a
ticket punch, paper rustle, a far-off theatre murmur), `petal` (8-bit blips, a tape deck's clunk, a VHS tape finding
its tracking), `glasshouse` (glass chimes, a 2000s start-up swell, soft wind). Every sound is synthesised from code
(`scripts/sounds/`, a seeded renderer, so the same source always gives the same sound), encoded to Ogg Opus, and kept
in the repo with its source. No sound is recorded from or modelled on another app or product.

**Which packs are on, and in what order.** Per device, from the top:
1. **One sound replaced** by the listener: any event can take any pack's sound for any event, or a file the listener
   adds, which the hub keeps in their own pack (`my-sounds`, an ordinary pack).
2. **The packs the listener added,** in their order (Settings › Sounds).
3. **The look's pack:** the theme's own if it has one, else the one of the design it extends.
4. **`default`.**

A sound comes from the first pack that has it. Surprises come from every pack that is on; two that declare the same
egg `name` are one egg, and the higher pack's declaration wins (so a design's pack gives lull's own `night-owl` its
own sound without a second owl). The listener can switch off any pack, any single sound and any single egg. So look,
sounds and surprises mix freely: one theme's look with another theme's sounds and a friend's surprises.

**API** (api.md has the details):
- `GET /api/packs`: every pack (built-in, the listener's, and each theme that carries one), each with its `rev`, its
  sounds as URLs and its eggs as declared.
- `POST /api/packs` (a `.lullpack`), `DELETE /api/packs/{id}`;
- `GET /api/packs/{id}/{path}` (immutable per `rev`), `PUT` of one file or of `pack.json` (the listener's own pack, a
  replaced sound).
- A theme's pack files are served with its other files (`GET /api/themes/{id}/...`).

### The settings

A Sounds section in Settings, **per device** like the other display settings (`localStorage['lull:sounds']`):
- **Sounds,** the master switch, and **Volume** (a share of the device's volume, 40% to start);
- a switch per sound, with a play button to hear it, and Replace;
- **The packs:** the look's, `default`, and any the listener added (importing one adds it), in order, each with a
  switch;
- **Only when no music is playing** (on): while music plays, only `play` and `pause` sound, at the press; with it
  off, the short sounds may play over music, quietly; the buffering loop never does;
- **Also when lull is in the background,** per sound, for `found`, `stalled`, `saved` and `error` only (off);
- **Follow the system's quiet settings** (on): no sound while the system asks for reduced motion (lull's own Motion
  setting, then the system's) or for quiet: Android in silent or vibrate mode, or with touch sounds off; Windows with
  the No Sounds scheme; GNOME with event sounds off. A browser can't read the system's sound settings, so there only
  reduced motion counts;
- **Hidden surprises** (on): the easter eggs below, their looks and their sounds, and behind a spoiler a switch for
  each one.

The master switch starts **on**, so a listener hears their design's world without looking for it; the volume starts
low.

### The rules

- **Never over the music.** "Music playing" means sound is coming out (`.is-playing-now`), not that a song is
  loaded. The buffering loop plays only in silence, and fades out within 60 ms of the song's first sound.
- **Short and quiet.** The lengths above, and lull levels every sound as it decodes it (one pass over its samples,
  once): its peak to at most -12 dBFS and its loudness to one quiet target, so no pack can be louder than another.
- **No sound on a reload.** `boot` plays once per app start (a marker in `sessionStorage`, which a reload keeps and a
  new start clears), and only if audio can start then (the desktop app, Android, an installed PWA); a browser tab that
  can't is not given its boot sound later. A state the page restores at start (a song that was buffering, a source
  being found) makes no sound: only changes that happen while the page is up do.
- **Nothing while lull isn't in front,** except the sounds the listener asked to hear in the background. In front
  means the page is visible and one of lull's windows has the focus: the desktop app tells `main` when its mini
  player, strip or Classic window does.
- **One at a time.** A new sound stops the one before; the same sound within 80 ms is dropped.
- **Classic is its own world:** `classic` plays as it opens, then lull's sounds rest until it closes.

### Cost

- **Nothing before it can sound, then loaded once, decoded once.** A browser plays no page's audio before the
  listener's first tap or key, so in a browser the engine (`web/sounds.js` and its rules, about 9 KB) loads on that
  first tap or key, and a tab nobody touches fetches none of it: the start pays only app.js' few lines that wait for the
  tap. The app and an installed PWA, where audio may start by itself, load it at the start for the start-up sound. The
  easter eggs come with it, so in a browser they begin at the first tap too; nothing but a clock egg could show before
  one, as playing needs a tap. The engine touches no DOM. It fetches the pack's files after the first screen has
  settled (the service worker keeps them), and decodes each once into a Web Audio buffer.
- **Its own Web Audio context,** apart from the player's: a cue never passes through the equaliser, the crossfade or
  the level, and nothing in the player waits for it. The context starts on the first gesture (autoplay rules) and
  suspends after 10 s without a sound, so an idle lull keeps no audio thread awake.
- **No redraws.** Playing a sound changes nothing on screen, so the frame, layer and list gates are untouched.
- **Android:** the page's Web Audio takes no audio focus, so the native player is never paused or ducked by a cue. A
  build that did would play cues through the player plugin instead; the Android check confirms it on a device.
- **The decisions are pure** (`web/sound-rules.js`: what plays, given the event, the player's state, the settings,
  focus and the system's quiet), tested with `node --test`. A UI gate (`gates/sounds.spec.mjs`) checks the real page:
  no sound on a reload, no sound while music plays, the buffering loop gone as the song starts, and no sound in a
  hidden page.

### Where it runs

The same everywhere, from the same files:
- **The web UI** in a browser and as a PWA.
- **The desktop app:** `main`'s page plays every sound, the mini player's presses included (they already run there);
  the app reports which of its windows has the focus. The pages the app shows without the hub (connect, unreachable)
  play none.
- **Android:** the page in the WebView plays them; the player plugin reports the system's quiet (ringer mode, touch
  sounds).

### Easter eggs: triggers and reactions a package declares

lull has no easter egg of its own in code. An egg is a declaration in a pack (`eggs` in `pack.json`): **when** its
conditions all hold, lull plays its **reaction**. lull's own surprises are the `default` pack's declarations, and
a design's pack re-declares the ones it gives its own sound or look; an author has every one of their powers.
Authors write no JavaScript (themes never run code), only these declarations, which the hub validates on import.

```json
{ "name": "night-owl",
  "when": [{ "clock": { "from": "03:00", "to": "04:00" } }, { "player": { "playing": true } }],
  "sound": "sounds/owl.ogg", "for": 600, "once": "night" }
```

**The conditions** (each with its parameters; an egg's `when` lists one or more, and all must hold):

| Condition | Parameters | Holds |
|---|---|---|
| `clock` | `from`, `to` (`HH:MM[:SS]`, the device's time; may wrap past midnight), `date` (`MM-DD`, or a list), `weekday` (`mon`...`sun`, or a list), `countdown` (true: `--egg-count` is the seconds left until `to`) | while the time is inside |
| `words` | `in` (`title`, `artist`, `album`; a list), `any` (1-40 words in any language and script; matched as whole words, ignoring case and accents, and inside the text for scripts written without spaces, such as Chinese, Japanese or Thai) | while a song whose words match is the current one; `data-egg-word` is the word |
| `repeat` | `times` (2-100) | the current song has played through `times` times in a row |
| `gesture` | `on` (`brand`, `cover`, `title`: the logo, the playing cover, the playing title), `taps` (2-12), `within` (s, at most 5) | at the tap that completes the count |
| `milestone` | `of` (`likes`, `follows`: what every device of the hub knows), `at` (a number or a list) | as the count reaches one of them; `--egg-count` is that number |
| `player` | `status` (a player status, or a list), `playing` (true: sound is coming out), `screen` (a `data-screen`), `idle` (s with no tap, key or scroll), `held` (s they have held together) | while all of them hold |
| `event` | `on` (a sound event: `boot`, `liked`...) | at that event |
| `chance` | `one_in` (2-1000) | once per check, with that chance (used with `event` or a moment's condition) |

**The reaction** (everything optional but `name`):
- `name`: the egg's look: `:root[data-egg~="<name>"]` while it is live, with `--egg-count` and `data-egg-word` where
  its conditions give one. The theme or pack styles it, in `theme.css` or `pack.css`, like any hook.
- `sound`: a file, played under the sound rules; `instead` (true): it takes the place of the sound of the event that
  fired it (one start in 40 gets a different `boot`), or of the buffering loop while the egg is live (static that
  slowly becomes a melody: only there may the sound be a loop, `{ "file", "loop": true }`).
- `slot`: `slots/egg.<name>.html` drawn into lull's `.egg-stage` (a layer above the page that takes no pointer and
  covers no control) while it is live, with the fields `count`, `word`, `title`, `artist`, `hour`, as other slots are.
- `line`: words lull shows in `.egg-line` (a quiet line in the design's own type, read out politely to a screen
  reader), with `{{count}}` and `{{word}}`.
- `go`: an in-app place to open (`#/credits`, lull's hidden credits: who made lull, its open parts, the packs'
  authors).
- `for`: how long it stays live once its conditions stop holding, or after a moment's (a tap, an event): 0.5 to 600 s,
  4 s to start.
- `once`: how often it may fire: `always`, `song`, `session`, `day`, `night` (18:00 to 06:00), `week`, `year` (at
  most once a `song` when not given).

**lull's own** (the `default` pack's):

| Egg | When |
|---|---|
| `night-owl` | a song playing between 3 and 4 in the morning, once a night |
| `new-year` | the last 10 s before midnight on 31 December (a countdown), then "Happy new year" |
| `birthday` | lull's birthday, 31 October, all day |
| `moon`, `rain`, `star`, `sun`, `snow`, `night`, `sea`, `storm`, `fire`, `dream` | a song with that word in its title, in a dozen languages and scripts, while it plays (one egg each, so a theme styles the idea whatever the language) |
| `still` | no input for 3 minutes while a song plays on Now Playing: the logo breathes slowly, until the next input |
| `on-repeat` | the same song played through 10 times in a row |
| `credits` | the logo tapped 7 times within 3 s: opens the credits |
| `milestone` | the 100th, 500th and 1,000th liked song |
| `long-wait` | buffering for 20 s in silence: the loop slowly gives way to a faint melody |
| `rare-boot` | one start in 40: a different start-up sound |

**Cheap by construction.** Conditions are checked on what already happens, never polled: a song change, a status
change, a like, a tap on the three gesture targets, and for `clock` one timer set for the next boundary any egg
names; `idle`, `held` and `for` are one timer each, reset by input. A reaction is a root attribute, a sound under the sound
rules, at most one small slot in a layer that takes no pointer, and a line of words; under reduced motion lull's base
CSS stills its motion like any other, and it can't play over the music or while lull isn't in front unless the sound
rules allow.

**Rare and never in the way.** Each egg fires at most once per its `once`, never while a dialog or sheet is open,
and ends at the next input. **Hidden surprises** in Settings › Sounds switches them all off; behind a "Show the
surprises" spoiler, each pack's eggs are listed with a switch each. **Theme Studio** lists the theme's eggs with
their errors, and fires any of them by hand in its preview. They behave the same on desktop, phone and web, and none
is a game.

## Alternatives

- **Sounds synthesised in the page** (Web Audio oscillators, no files). Nothing to download, but a theme can't ship
  one without code, which themes never run.
- **Sounds and eggs in `theme.json`.** One file fewer, but a standalone pack would need a second manifest format; one
  `pack.json` for both keeps a single validator and lets a pack move into a theme as it is.
- **Easter eggs as a fixed list lull names, that themes can only answer.** Simpler to build, but nobody but lull could
  make one, and lull's own would use powers no author has. Declared triggers give authors exactly lull's powers.
- **Eggs as scripts.** Any trigger an author can think of, but themes never run code: a shared pack could read the
  listener's data.
- **`<audio>` elements per sound.** Simple, but each one is a media player: it can take the media session and audio
  focus from the music on Android and in the desktop app, and it decodes again on every play.
- **Any audio format the browser plays.** Wider choice, but the hub couldn't check lengths without decoding, and MP3
  and WAV are larger for the same sound. Ogg Opus and Vorbis are small, and their lengths are in their headers.
- **Sounds off by default.** Surprises nobody, but most listeners would never find their design's world; the low
  starting volume and the silence rule are what keep it from being in the way.

## Consequences

- **The theme API grows, and its version stays 1:** `pack.json`, `pack.css`, `sounds/` and `slots/egg.<name>.html`
  in a package, the sound events, the egg conditions and reactions, `:root[data-egg]`, `data-egg-word`,
  `--egg-count`, `.egg-stage` and `.egg-line`. Renaming any of them is a breaking change.
- **docs/themes.md** gets a section on packs, sounds and easter eggs, and **api.md** the new endpoints and the
  `pack.changed` event.
- **The UI gates** gain `gates/sounds.spec.mjs`; the perf gates need no change, since a sound draws nothing.
- **The desktop app** reports focus across its windows, and the system's sound scheme on Windows and GNOME; the
  **Android player plugin** reports the ringer mode and touch sounds.
- **A built-in design's sound world** is its pack in `web/packs/<design>/`, not in its package, so the designs and
  their sounds change independently; a design's own easter-egg looks can live in its pack's `pack.css`.
- **Not in Safari before it decodes Ogg:** there a sound that won't decode stays silent, and the rest of lull is
  unchanged.
