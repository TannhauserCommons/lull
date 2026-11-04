/* The player's rules (docs/context/web.md#player): what the queue moves to, what a play records, how loud, when the
   next song loads, what the hub preloads, when radio adds songs, how far a seek may go (and how long a kept one waits)
   and when a better copy is switched to. The page's player (app.js) decides by these, and so does the desktop's native Classic player
   (app/src/desktop/classic_native), which restates them in Rust: tests/player-rules.json holds the cases both run
   (tests/web/player-rules.test.mjs here, the engine's tests there), so a rule changed on one side fails until the
   other matches. Pure functions over plain values, so node can test them. */
(function (root) {
  'use strict';
  var R = {};

  /* timings, in ms or s as named */
  R.TICK_MS = 250;              // the player's clock
  R.SKIP_FAILED_MS = 2500;      // a song no peer delivers gives way to the next after this
  R.PREFETCH_MS = 800;          // preloading waits this long after the last change
  R.LISTEN_GAP_S = 1.5;         // a step of the position this long or longer isn't listening
  R.PREVIOUS_RESTARTS_S = 3;    // Previous past this restarts the song
  R.GAPLESS_WITHIN_S = 2;       // gapless continues only when the song ended this close to its end
  R.ENDED_EARLY_S = 2;          // a Soulseek response ending further from the end is a fallback in progress
  R.RADIO_AHEAD = 3;            // radio keeps this many songs after the playing one
  R.RADIO_STEER = 0.4;          // "radio when the queue ends" steers this far
  R.LOUDNESS_AFTER = 12;        // measured loudness levels after this many readings (one a tick: 3 s)
  R.LOUDNESS_MIN_DB = -12;
  R.LOUDNESS_MAX_DB = 6;
  R.SWITCH_LEAD_S = 45;         // a better copy needs this much of the song left
  R.SWITCH_AHEAD_S = 20;        // and to have arrived this far past where the song is
  R.SWITCH_TRIES = 3;
  R.SWITCH_FADE_S = 0.2;
  R.SWITCH_QUIET_S = 3;         // the quiet moment is waited for this long at most
  R.SWITCH_GLIDE_S = 1.5;       // after the fade, the copy's level glides to its own with this time constant

  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }

  /* the index the queue moves to next: repeat one (only when a song ends by itself), the next entry, wrap, or -1.
     q: { index, length, repeat: off | all | one } */
  R.nextIndex = function (q, auto) {
    if (auto && q.repeat === 'one') return q.index;
    if (q.index + 1 < q.length) return q.index + 1;
    return q.repeat === 'all' ? 0 : -1;
  };
  /* Previous: past 3 s, or on the first song, the song starts again (-1); otherwise the index before */
  R.previous = function (index, pos) { return pos > R.PREVIOUS_RESTARTS_S || index === 0 ? -1 : index - 1; };
  R.cycleRepeat = function (mode) { return { off: 'all', all: 'one', one: 'off' }[mode]; };

  /* plays: the seconds a step of the position adds to what was heard */
  R.heard = function (step, playing) { return step > 0 && step < R.LISTEN_GAP_S && playing ? step : 0; };
  /* the body that starts a play (POST /api/plays) and the one that ends it (PATCH); how: completed (ended by itself) |
     skipped (the listener moved on) | stopped (closed) */
  R.playStart = function (trackId, duration, recommended) {
    var body = { trackId: trackId, recommended: !!recommended };
    if (duration) body.duration = Math.round(duration);
    return body;
  };
  R.playEnd = function (heard, how) { return { playedS: Math.round(heard), completed: how === 'completed', skipped: how === 'skipped' }; };

  /* how loud: the output's volume (a perceptual curve), and a song's level: ReplayGain in Settings > Playback's mode,
     or its measured loudness levelled toward the target */
  R.outputVolume = function (volume, muted) { return muted ? 0 : volume * volume; };
  R.replayGainDb = function (replayGain, mode) {
    if (!replayGain || mode.indexOf('replaygain') !== 0) return null;
    return mode === 'replaygain-album' ? replayGain.album : replayGain.track;
  };
  R.gain = function (db) { return Math.pow(10, db / 20); };
  /* the level for a song measured at `rms` (null: keep the level it has) */
  R.loudness = function (targetDb, rms, readings) {
    if (readings < R.LOUDNESS_AFTER || rms < 1e-4) return null;
    return clamp(R.gain(targetDb) / rms, R.gain(R.LOUDNESS_MIN_DB), R.gain(R.LOUDNESS_MAX_DB));
  };

  /* gapless and crossfade: repeat one never crossfades; the next song is set once the playing one is within `lead`
     of its end (a Soulseek song still transferring keeps the bandwidth to itself until its last half minute) */
  R.crossfadeS = function (repeat, crossfade) { return repeat === 'one' ? 0 : crossfade || 0; };
  R.preloadLead = function (crossfade, soulseek, received) {
    return Math.max(30, (crossfade || 0) + 15) + (soulseek && received < 1 ? 0 : 60);
  };
  R.preloadNow = function (lead, duration, pos) { return !!duration && duration - pos <= lead; };
  /* the output moves into the next song only when the playing one really reached its end */
  R.gapless = function (duration, pos) { return !duration || duration - pos <= R.GAPLESS_WITHIN_S; };
  /* a Soulseek response that ends early is a fallback in progress (buffering), not the song's end */
  R.endedEarly = function (soulseek, complete, duration, pos) { return soulseek && !complete && duration - pos > R.ENDED_EARLY_S; };
  /* the crossfade's levels at k (0-1) of the way, equal power; it lasts what is left of the song, at least 1 s */
  R.fade = function (k) { return [Math.cos(k * Math.PI / 2), Math.sin(k * Math.PI / 2)]; };
  R.crossfadeLengthS = function (left) { return Math.max(1, left); };

  /* preloading: the next few songs the hub fetches ahead. q: { index, ids (the queue's track ids), shuffle, repeat };
     isSaved(id): the listener saved it (it needs no fetching); n: Settings > Playback's count */
  R.upcoming = function (q, isSaved, n) {
    if (n <= 0 || q.repeat === 'one') return [];
    var next = R.nextIndex({ index: q.index, length: q.ids.length, repeat: q.repeat }, true), ids = [], current = q.ids[q.index];
    var at = q.shuffle ? [next] : q.ids.map(function (id, i) { return i; }).slice(q.index + 1);
    at.forEach(function (i) {
      var id = q.ids[i];
      if (id != null && ids.length < n && !isSaved(id) && ids.indexOf(id) < 0 && id !== current) ids.push(id);
    });
    return ids;
  };

  /* radio: with "radio when the queue ends" on, repeat off and nothing after the playing song, the playing song seeds
     a radio; a radio that keeps adding asks for more while fewer than three songs follow.
     q: { index, length, repeat, radio: null | { keepsAdding } }; -> { seed (start one), ask } */
  R.radio = function (q, radioWhenQueueEnds) {
    var left = q.length - q.index - 1;
    var seed = !q.radio && radioWhenQueueEnds && q.repeat === 'off' && q.length > 0 && left === 0;
    var active = seed || !!(q.radio && q.radio.keepsAdding);
    return { seed: seed, ask: active && left < R.RADIO_AHEAD };
  };

  /* seeking a Soulseek song still arriving (0-1 of the song). buffered: [[from, to]] the output holds; received: the
     transfer's share. A seek inside what arrived (less a margin, so the output doesn't stall at once) lands now. Past
     it, with Settings > Playback's "Jump to where I seek once it has downloaded" on (waits), the song plays on where
     it is and the seek is kept: asked again as more arrives, it lands once the limit reaches it. Off, it lands just
     inside what arrived */
  R.SEEK_MARGIN = 0.01;
  R.seekLimit = function (soulseek, buffered, received) {
    if (!soulseek) return 1;
    var end = buffered.reduce(function (m, r) { return Math.max(m, r[1]); }, 0);
    return Math.max(end, received >= 1 ? 1 : received - R.SEEK_MARGIN);
  };
  /* -> { fraction, how: now | waits | snapped } */
  R.seekTo = function (fraction, limit, waits) {
    fraction = clamp(fraction, 0, 1);
    if (fraction <= limit) return { fraction: fraction, how: 'now' };
    if (waits) return { fraction: fraction, how: 'waits' };
    return { fraction: Math.max(0, limit - 0.004), how: 'snapped' };
  };
  /* about how long a kept seek waits, in whole seconds rounded up: the bytes still to come before it lands, at the
     transfer's speed (bytes a second); null while the speed isn't known */
  R.seekWaitS = function (fraction, received, size, speed) {
    var left = (Math.min(1, fraction + R.SEEK_MARGIN) - received) * size;
    if (left <= 0) return 0;
    return speed > 0 ? Math.ceil(left / speed) : null;
  };

  /* a better copy (docs/context/web.md#better-copies), offered while a song plays: switch | wait | next-play.
     s: { picked (a copy the listener chose), canSwitch (the output can line copies up), steady (playing, sound
     flowing, no crossfade), duration, pos, arrived (0-1 of the copy) } */
  R.switchNow = function (s) {
    if (s.picked) return 'wait';
    if (!s.canSwitch) return 'next-play';
    if (!s.steady) return 'wait';
    if (s.duration && s.duration - s.pos < R.SWITCH_LEAD_S) return 'next-play';
    if (s.arrived < 1 && (!s.duration || s.arrived * s.duration < s.pos + R.SWITCH_AHEAD_S)) return 'wait';
    return 'switch';
  };
  /* a quiet moment to switch in: the level now well under the median of the last ones, once there are eight */
  R.quiet = function (levels) {
    if (levels.length < 8) return false;
    var median = levels.slice().sort(function (x, y) { return x - y; })[levels.length >> 1];
    return levels[levels.length - 1] <= 0.6 * median;
  };
  /* the copy's level matched to the song's for the fade (it then glides to its own) */
  R.levelMatch = function (song, copy) { return clamp(song / Math.max(copy, 1e-6), 0.5, 2); };

  root.LullRules = R;
  if (typeof module === 'object' && module.exports) module.exports = R;
})(typeof self !== 'undefined' ? self : this);
