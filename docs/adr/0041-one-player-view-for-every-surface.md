# ADR-0041: One player view for every surface

- Status: accepted (the owner approved the plan on 2026-11-16)
- Date: 2026-11-16

## Context

The desktop mini player's position moved only when the listener paused (desktop app 0.2.10, fixed for the position in
075410a3). Looking for its siblings found the cause is wider than the mini player. The player's state reaches the
screen along about ten paths, each with its own trigger:
- the player bar's markup (`renderBar`), its seeks and times (`updateSeeks`), the rows (`syncRows`), Now Playing's
  own painters, the lull-parts redraw, the live model's writer (`designs/live.js`), the tray's `set_now_playing`,
  Media Session, Connect's reports, the source details sheet;
- the mini player windows, which repaint from a snapshot `main` sends, through `paintPlayer`: a hand-kept copy of the
  bar's painter ("as the web UI's bar has them") that nothing derives from the bar.

Whether the song is "playing now" is decided by seven rules that disagree (`P.heard`, the mini's `runs`, the wave's
`moving`, the play button's `playingNow`, Now Playing's `playingNow`, the bar's data-playing, Connect's heartbeat), and
the set of "waiting" statuses is written out six times with different members. Every new fact has to be wired into
each path by hand, and nothing checks that it was. Confirmed results:
- the mini player draws the peer's percentage once and keeps it for the whole download, and its format chip is the
  catalogue's, not the copy that plays (the bar's);
- four of the five designs show no "finding / buffering / failed" in the mini player;
- the mini player freezes for good once `main` reloads (a crash, the hub-unreachable page, a reconnect): `main` knows
  a mini player is open only from the one `hello` it sent when it opened;
- a mini player from an older app and a `main` from a newer hub (or the other way round) talk past each other, and
  neither notices: the snapshot has no version;
- Connect never reports a song that failed, so other devices show it playing with a moving clock;
- when the system pauses the output (another app takes the audio, headphones out), lull goes on showing it playing:
  `ps.playing` is what the listener asked for, and nothing follows the output's own pause into it;
- Home's lead slot keeps the song it was drawn with after the song changes.

## Decision

**One view, derived in one place.** `web/player-view.js` (`window.PlayerView`, also a node module, like
`player-rules.js`) holds pure functions over plain values. `PlayerView.derive(input)` turns the player's state into
the view every surface shows:

```
{ v,                        // the wire version, PlayerView.VERSION
  activity,                 // idle | loading | choose | playing | paused | ended | error
  status, words,            // ps.status and P.stateWords(), as today
  track,                    // { id, title, artist, album } or null
  copy,                     // what P.playingCopy() says plays: { where, format, lossless, peer, received } or null
  clock,                    // { pos, at, rate, duration, known }: pos seconds at time at (ms), moving at rate (0 or 1)
  transfer,                 // { received, front, parts, transferring, buffered }: buffered is the end the output holds (0-1)
  seekWait,                 // the fraction a pending seek lands at, or null
  liked, saved, shuffle, repeat, volume, muted, crossfading }
```

`activity` is the one answer to "what is the player doing", by this table, first match:

| activity | when |
|---|---|
| `idle` | no song, or status `idle` |
| `error` | status `failed` or `jellyfin-error` |
| `ended` | status `ended` |
| `choose` | status `choose` (waiting for the listener to pick a copy) |
| `paused` | the listener's intent is paused |
| `playing` | the output plays and isn't waiting, whatever the status says about the source (a stalled peer whose arrived part still plays is playing) |
| `loading` | anything else: status `finding`, `queued`, `buffering`, `stalled`, `streaming` or `library` with no sound yet (no `time`, or one with `playing: false` or `waiting: true`) |

The status says why; the sound says whether. A view that called a song loading while it was heard would stop the
wave, the clock and Connect at every stalled peer the next copy covers.

`clock.rate` is 1 only while `activity` is `playing`. Every surface places the song with `PlayerView.posAt(view, now)`,
never by extrapolating on its own. `PlayerView.key(view)` is the view without `clock.pos` and `clock.at`, and
`PlayerView.strayed(prev, next, now)` says the clock is a second or more off. Together they decide when a view is new:
the position never makes it new while it runs where the clock says.

**One writer for the player's hooks.** `PlayerView.paint(scope, view, now)` is the only code that writes a hook whose
value changes during a song, in the page and in the mini player windows alike. These are the player hosts'
`data-playing`, `data-status`, `data-status-text`, `--status-text`, `--song-pos`, `--rx`; the seek and wave sliders
(`[data-seek="player"]`, the mini's `[data-dk="seek"]`), their classes and aria values; `[data-bind]` pos, dur,
buffered-to, received; `[data-dk-pos]`, `[data-dk-dur]`, `[data-dk-vol]`; the play, like, shuffle and repeat keys
(`[data-bind]` and `[data-dk]` alike); the volume slider.
- The hook names stay as docs/themes.md lists them, so no theme changes.
- Markup builders (the bar, the mini's `nowMarkup`, slots) draw the song: title, artist, cover, the copy's format. They
  leave state to `paint`, so nothing drawn once can go stale. The peer's percentage becomes a `[data-bind="received"]` hook, as on the page.
- The mini player's default markup and each design's `miniPlayer` slot gain a status line, `[data-dk-status]`, whose
  text `paint` writes from `words`, so every design says what the player is doing.

**Published when it changes, not when a window is visible.** `App.player.view()` derives the view from the engine.
The engine publishes it as the `view` event when its key changes or its clock strays, from `changed()` and from the
player's clock before the clock's `document.hidden` early return. So a hidden page still publishes. The page paints
on `view`, and on its visible clock between views. Every part that shows the activity (the play key and its
spinner, the wave, Now Playing's light) repaints on `view`: a song can go from loading to playing with no other
player event. The live model's player values (`song.pos`, `song.elapsed`,
`song.buffered`, `song.received`) are read from the view. Connect reports whenever the view's activity changes, so a failure, an end or an outside pause reaches other devices
at once. Its `playing` stays the listener's intent (every terminal state clears the intent, and a device taking over
plays on what the listener there wanted, loading or not).

**The mini player's link has a version and two ends that each say hello.**
- `main` sends the view itself as the `player` event, and the cover on its own as today (`player-art`).
- The mini player's `hello` carries `v`. `main` emits `main-hello` when its page starts, and the mini player answers
  with `hello` again, so a reloaded `main` learns it is open.
- `main` emits `main-bye` on `pagehide`, and the app emits it when `main`'s renderer dies. The mini player then
  stops its clock and shows that lull's window is away until a view comes.
- A `hello` or a view whose `v` isn't the mini player's own is refused loudly. The mini player says which side needs
  updating and stops its clock, instead of drawing a shape it doesn't know.
- `PlayerView.VERSION` is a whole number from 1, raised on any change to the view's shape. `main` answers every
  `hello` with its view, whatever the `v`; the mini player is the side that judges. `main-hello` and `main-bye` carry
  `{}`.
- The link's state is `data-link` on `[data-dk-player]`: `ok`, `away` (after `main-bye`, until a view comes),
  `update-app` (`main` is newer) or `update-hub` (`main` is older, or sent a snapshot with no `v`).

**The listener's intent follows the output.** An output that pauses or resumes for a reason of the system's (another
app took the audio focus, headphones out, the lock screen's or a headset's keys) says so: the output interface's
`system` event, `{ key, playing }`. The engine sets `ps.playing` to it, as any player shows a pause the system made.
The engine never guesses one from a `time` with `playing: false`: an output reports that at a song's end, when it is
cleared and on a seek after the end too, and a guessed pause nothing ever undid broke repeat-one, peer fallbacks and
skipping a failed song. The browser output sends `system` for an element `pause` or `play` that its own `pause()` or
`play()` didn't ask for (and not for the pause an element makes at its end). The Android output sends the media
session's play and pause to the page as the `remote` commands next and previous already are, so they set the intent
through `P.play`/`P.pause`, and sends `system` for a play-when-ready change for audio focus or noise.

**Every slot field stays current.** A slot's `{{fields}}` are drawn when it draws. A slot is drawn again when a value
of its pace changes (docs/themes.md section 14: a song-pace field on a new song, next song or status), on every
screen that shows it, not only where one screen happens to redraw.

**Checked by gates, per design and per surface.** One gate plays, skips, likes, pauses from the system and fails a
song with `main` hidden through `visibilitychange`, not only `document.hidden`. In every design and both mini styles
it checks that each changing value repaints: the bar, Now Playing, Home's lead, and the mini player fed by `main`'s
real views. `tests/web/player-view.test.mjs` pins `derive`'s table and the clock.

**No switch (ADR-0096).** This replaces code paths that already ship, so a switch would keep both paths and
reintroduce the drift it removes. It lands in steps, each green on its own: the view and its readers first, then the
painter and the mini player's link. A step moves a golden only where it fixes a stale value, and says so in its
commit.

## Consequences

- One rule for "playing now" and one writer per hook. A new fact is added to `derive` and `paint` once and every
  surface shows it. A test of `derive` covers what seven rules covered unevenly.
- `player-view.js` loads at the start in the page and in the app's bundled pages (`app/build.rs` stages it). It
  replaces code in `app.js` and `desktop.js` of about its size, and any growth is measured against the shipped-bytes
  budget (quality.md).
- The mini player and `main` must share `PlayerView.VERSION`. Changing the view's shape raises it, and an app or hub
  that lags shows a message instead of a frozen or wrong player. The 0.2.10 app predates the version and can't be
  told; its own updater is its fix.
- `PlayerView.pressed(view)` is what the play key shows as Pause: the intent while the song is neither over nor
  failed (`playing`, `loading`, `choose`), so the key always does what it shows.
- `data-playing` keeps its meaning (the listener's intent) for themes. `activity` is not a theme hook until a theme
  needs it.

## Alternatives

- **Patch each stale surface where it was found:** six fixes in six places, and the seventh path drifts next time.
  This is the pattern that produced the bugs.
- **Keep the mini player's own painter and send it more fields:** two writers that must agree by hand. `paintPlayer`
  already disagrees with the bar on the format and the peer's percentage.
- **Send the position to the mini player every second instead of a clock:** more bridge traffic. It still freezes
  when `main` stops sending (hidden, throttled, reloaded), and doesn't fix the six other derivations.
- **A keep-alive from `main` instead of `main-hello`/`main-bye`:** a timer in both windows and a timeout to tune,
  where the two lifecycle events say exactly when `main` comes and goes.
