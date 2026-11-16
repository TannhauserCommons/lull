/* The one player view (ADR-0041, docs/context/web.md#player-engine): what every surface (the bar, Now Playing, Home's lead,
   the live model, Connect, the mini player windows) shows of the player. derive() turns the player's plain state into the
   view, so "what is the player doing" has one answer: `activity`. The page's player (app.js) publishes it as the `view`
   event when its key changes or its clock strays; every surface places the song with posAt(), never by extrapolating on
   its own. Pure functions over plain values, so node can test them (tests/web/player-view.test.mjs). */
(function (root) {
  'use strict';
  var V = {};

  /* raised on any change to the view's shape: the mini player windows and `main` must share it (ADR-0041) */
  V.VERSION = 1;
  V.STRAY_S = 1;                // a clock this far (or farther) from where the last view said it would be has strayed

  /* the statuses the player has (app.js, "Player engine"); an unknown one fails loudly rather than guessing */
  var SOURCE = { finding: 1, queued: 1, buffering: 1, stalled: 1, streaming: 1, library: 1 };

  /* what the player is doing, by the first row that matches (ADR-0041's table) */
  function activityOf(i) {
    if (!i.track || i.status === 'idle') return 'idle';
    if (i.status === 'failed' || i.status === 'jellyfin-error') return 'error';
    if (i.status === 'ended') return 'ended';
    if (i.status === 'choose') return 'choose';
    if (!SOURCE[i.status]) throw new Error('Unknown player status: ' + i.status);
    if (!i.intent) return 'paused';
    /* the status says why, the sound says whether: a stalled peer whose arrived part still plays is playing */
    return i.sound && i.sound.playing && !i.sound.waiting ? 'playing' : 'loading';
  }

  V.derive = function (i) {
    var activity = activityOf(i), t = i.track, c = i.copy;
    return {
      v: V.VERSION,
      activity: activity,
      status: i.status,
      words: i.words,
      track: t ? { id: t.id, title: t.title, artist: t.artist, album: t.album } : null,
      copy: c ? { where: c.where, format: c.format, lossless: !!c.lossless, peer: c.peer || null, received: c.received } : null,
      clock: { pos: i.pos, at: i.now, rate: activity === 'playing' ? 1 : 0, duration: i.duration, known: !!i.durationKnown },
      transfer: { received: i.received, front: i.front, parts: i.parts, buffered: i.buffered && i.buffered.length ? i.buffered[i.buffered.length - 1][1] : 0,
        transferring: i.received < 1 && (i.status === 'streaming' || i.status === 'buffering') },
      seekWait: i.seekWait ? i.seekWait.fraction : null,
      liked: !!i.liked, saved: !!i.saved, shuffle: !!i.shuffle, repeat: i.repeat, volume: i.volume, muted: !!i.muted,
      crossfading: !!i.crossfading
    };
  };

  /* where the song is at `now` (ms): on from the clock's own time while it moves, never past the song's end */
  V.posAt = function (view, now) {
    var c = view.clock, pos = c.pos + c.rate * (now - c.at) / 1000;
    return c.duration > 0 ? Math.min(pos, c.duration) : pos;
  };

  /* the view without where its clock says the song is: a view is new when this changes, not as the clock runs */
  V.key = function (view) {
    var c = view.clock;
    return Object.assign({}, view, { clock: { rate: c.rate, duration: c.duration, known: c.known } });
  };

  /* the clock is off: `next` puts the song a second or more from where `prev`'s clock had it by `now` (a seek, a stall,
     a drifting output), so the surfaces that run on `prev` need `next` */
  V.strayed = function (prev, next, now) {
    return !prev || Math.abs(V.posAt(next, now) - V.posAt(prev, now)) >= V.STRAY_S;
  };

  /* the play key shows Pause while the listener's play stands: the song plays, waits for sound, or waits for a copy to
     be picked. A paused, ended or failed song shows Play, so the key always does what it shows */
  V.pressed = function (view) { return view.activity === 'playing' || view.activity === 'loading' || view.activity === 'choose'; };

  root.PlayerView = V;
  if (typeof module === 'object' && module.exports) module.exports = V;
})(typeof self !== 'undefined' ? self : this);
