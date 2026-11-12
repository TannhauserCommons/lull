/* The player's rules (docs/context/web.md#player): what the queue moves to, what a play records, how loud, when the
   next song loads, what the hub preloads, when radio adds songs, what of a song can play, where a seek lands (or is kept, or fetched), how long a kept one waits
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

  /* seeking a Soulseek song still arriving (0-1 of the song). arrived: [[from, to]] the parts the hub has (one from 0,
     until a seek ahead starts the transfer elsewhere, ADR-0014); buffered: what the output holds. What can play now is
     those parts, each less a margin at its end so the output doesn't stall at once (a part reaching the song's end keeps
     it). A seek inside it lands now. Past it: when the hub can start the transfer there (aheadS, its `seekAheadS`)
     and the transfer running would take longer than that to get there (waitS; unknown counts as longer), it lands now
     and the hub fetches from there; otherwise (sooner, or a format the hub can't start mid-file) the song plays on and
     the seek is kept, asked again as more arrives */
  R.SEEK_MARGIN = 0.01;
  R.playable = function (soulseek, arrived, buffered) {
    if (!soulseek) return [[0, 1]];
    var merged = [];
    arrived.concat(buffered).map(function (r) { return [r[0], r[1]]; }).sort(function (x, y) { return x[0] - y[0]; })
      .forEach(function (r) {
        var last = merged[merged.length - 1];
        if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]); else merged.push(r);
      });
    return merged.map(function (r) { return [r[0], r[1] >= 1 ? 1 : r[1] - R.SEEK_MARGIN]; })
      .filter(function (r) { return r[1] > r[0]; });
  };
  /* -> { fraction, how: now | fetches | waits } */
  R.seekTo = function (fraction, playable, waitS, aheadS) {
    fraction = clamp(fraction, 0, 1);
    var how = playable.some(function (r) { return r[0] <= fraction && fraction <= r[1]; }) ? 'now'
      : aheadS != null && (waitS == null || waitS > aheadS) ? 'fetches' : 'waits';
    return { fraction: fraction, how: how };
  };
  /* about how long the transfer takes to bring a seek at `fraction` (and the margin), in whole seconds rounded up: the
     bytes from the end of the part arriving toward it, at the transfer's speed (bytes a second); null while the speed
     isn't known */
  R.seekWaitS = function (fraction, arrived, size, speed) {
    var to = Math.min(1, fraction + R.SEEK_MARGIN);
    var from = arrived.reduce(function (m, r) { return r[0] <= to ? Math.max(m, Math.min(r[1], to)) : m; }, 0);
    var left = (to - from) * size;
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

  /* The copy that plays, for every place that says so (the player bar, Now Playing, source details; the page only, so
     tests/web/playing-copy.test.mjs holds its cases, not player-rules.json). Where it comes from is read from where the
     bytes come from (e: the queue entry, once its source is known), never from a peer not having spoken yet.
     s: { track, e: { local, soulseek, url }, status, peer: { name, format } | null, saved, received (0-1), library: folder | jellyfin | null }
     -> { where: device | library | soulseek | youtube, name (what to call the place; null for Soulseek until a peer answers),
          library, peer, format (null when unknown: show no chip), received, saved } */
  /* a song's `stream` event against the job followed: 'adopt' (the first, or a job naming it in `replaces`), 'follow',
     or 'drop' (another fetch of the song) */
  R.followEvent = function (job, ev) {
    if (!job) return 'adopt';
    if (ev.jobId === job) return 'follow';
    return ev.replaces && ev.replaces === job ? 'adopt' : 'drop';
  };

  /* the output failed (docs/context/web.md#a-songs-outcome; the page only, so tests/web/output-error.test.mjs holds its
     cases): 'restart' a song restored after a reload from 0:00 (its resumed stream may be gone), 'unpick' a copy the
     listener picked for lull's own choice (a pick stays with its queue entry, and may outlive the copy: the hub no
     longer has it, or this device can't play it), else 'ask-hub' for a Soulseek or YouTube song's outcome, or 'own'.
     s: { resuming, soulseek, picked } */
  R.afterOutputError = function (s) {
    if (s.resuming) return 'restart';
    if (!s.soulseek) return 'own';
    return s.picked ? 'unpick' : 'ask-hub';
  };

  R.playingCopy = function (s) {
    var e = s.e || {}, peer = s.peer || null, lib = s.library || null, where;
    if (e.local) where = 'device';
    else if (e.soulseek) where = peer && peer.name === 'YouTube' ? 'youtube' : 'soulseek';
    else if (e.url) where = 'library';
    else where = s.status === 'library' || s.status === 'jellyfin-error' ? 'library' : 'soulseek';   // not resolved yet
    var own = where === 'library' || where === 'device';
    return {
      where: where, library: lib, peer: own ? null : peer, saved: !!s.saved, received: s.received || 0,
      format: (!own && peer && peer.format) || s.track.format || null,
      name: where === 'device' ? 'this device' : where === 'library' ? (lib === 'jellyfin' ? 'Jellyfin' : lib === 'folder' ? 'your music folder' : 'your library') : peer ? peer.name : null
    };
  };

  root.LullRules = R;
  if (typeof module === 'object' && module.exports) module.exports = R;
})(typeof self !== 'undefined' ? self : this);
