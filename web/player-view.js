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

  /* the buffering sound's rule, as it always was, read from the view: it plays while the intent is on, nothing is heard and the
     status is finding, queued or buffering */
  var WAIT = { finding: 1, queued: 1, buffering: 1 };
  V.waitsForSource = function (view) { return view.intent && view.activity !== 'playing' && !!WAIT[view.status]; };

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
      since: i.since == null ? null : i.since,
      intent: !!i.intent,
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

  /* the play key shows Pause while the listener's play stands: the view's intent, which is what the key's press acts on,
     so the key always does what it shows. Every end and failure clears the intent, so those show Play */
  V.pressed = function (view) { return !!view.intent; };

  /* ---------- the painter ----------
     paint() is the one writer of every player hook whose value changes during a song, in the page and in the mini player
     windows alike. The markup builders draw the song and leave state to it. A write skips an unchanged value: the clock
     runs several times a second, and rewriting the same text, attribute or custom property still restyles what it
     touches (text changes the existing node's data: replacing the node makes the designs' page-wide :has() rules
     restyle the whole page each second). */
  /* how a surface finds its parts: the page replaces this with a finder that keeps its answers between changes
     (app.js: a long playlist's thousands of elements make every query cost) */
  V.find = function (scope, sel) { return Array.prototype.slice.call(scope.querySelectorAll(sel)); };

  V.time = function (sec, estimated) {
    sec = Math.max(0, Math.round(sec || 0));
    return (estimated ? '~' : '') + Math.floor(sec / 60) + ':' + String(sec % 60).padStart(2, '0');
  };
  /* words as a CSS string, for content: (a custom property holding them) */
  V.cssString = function (text) { return '"' + String(text).replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\A ') + '"'; };

  function setText(el, v) {
    var n = el.firstChild;
    if (n && n.nodeType === 3 && !n.nextSibling) { if (n.data !== v) n.data = v; }
    else if (el.textContent !== v) el.textContent = v;
  }
  function setAttr(el, name, v) { v = String(v); if (el.getAttribute(name) !== v) el.setAttribute(name, v); }
  function setVar(el, name, v) { v = String(v); if (el.style.getPropertyValue(name) !== v) el.style.setProperty(name, v); }
  function setClass(el, name, on) { if (el.classList.contains(name) !== on) el.classList.toggle(name, on); }
  V.setText = setText;
  function clamp(x) { return Math.max(0, Math.min(1, x)); }

  /* a key's icon follows the player: an icon lull drew in it (a <use> of lull's sprite, #i-...) is pointed at the new one;
     an empty play-pause or like key of the mini player (`[data-dk]`, `fill`) gets lull's, as docs/themes.md section 6 promises
     of those two. A theme's own markup is never written over: its words, an icon it draws itself, a key it draws with CSS (an
     empty repeat, shuffle or save key is one) */
  var LULL_ICON = '#i-';
  function ownIcon(u) { return (u.getAttribute('href') || '').indexOf(LULL_ICON) !== 0; }
  function keyIcon(b, name, cls, fill) {
    var u = b.querySelector('use'), svg = b.querySelector('svg.ic');
    if (u) { if (ownIcon(u)) return; setAttr(u, 'href', LULL_ICON + name); if (svg) setClass(svg, 'ic-play', cls === 'ic-play'); }
    else if (fill && b.hasAttribute('data-dk') && !b.textContent.trim() && !b.firstElementChild) b.innerHTML = window.icon(name, null, cls);
  }

  /* the parts that carry the player's state: the player bar, the mini player and a mini player window */
  var HOSTS = '#pbar, #mini, [data-dk-player]';
  /* the parts a design draws the song's place on (docs/themes.md: --song-pos). It isn't inherited (components.css): a value
     every element inherits restyles all of them at each step, a long playlist's thousands or a hero's hundreds */
  var SONG_POS_AT = HOSTS + ', .sky-specks, .now__dev, .now__devc, .disc-lead, [data-live]';
  V.SONG_POS_AT = SONG_POS_AT;   // live.js leaves --song-pos on these to the painter: one writer
  var LIKED_AT = HOSTS + ', .now__dev, .now__devc';
  /* the player's seek parts: the page's sliders, and inside the mini player the seek line and every wave (a theme may build
     its own into its mini player). A seek line or wave outside the player is the theme's own, never moved with the song */
  var SEEKS = '.seek[data-seek="player"], [data-dk-player] .seek--line', WAVES = '.wave[data-seek="player"], [data-dk-player] .wave';
  /* Now Playing's pills, as lull drew them before data-bind: a theme's markup copied from them carries data-act and the class */
  var SHUFFLE_KEYS = '[data-bind="shuffle"], [data-dk="shuffle"], .now .device__pill--shuffle';
  var REPEAT_KEYS = '[data-bind="repeat"], [data-dk="repeat"], .now .device__pill--repeat';
  var REPEAT_LABEL = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' };

  /* what paint() writes, as one selector: markup that carries any of it is drawn with the song's state at once (app.js patch) */
  V.HOOKS = [SONG_POS_AT, SEEKS, WAVES, '.seek-row', '.seek[data-seek="volume"]', '[data-bind]', '[data-dk]', '[data-dk-status]', '[data-dk-pos]',
    '[data-dk-dur]', '[data-dk-vol]', '[data-dk-play-label]', '[data-now="a-label"]', '[data-now="repeat"]', '.device__pill--shuffle', '.device__pill--repeat'].join(', ');

  V.paint = function (scope, view, now) {
    var find = function (sel) { return V.find(scope, sel); };
    var c = view.clock, tr = view.transfer, t = view.track, idle = view.activity === 'idle';
    var pos = V.posAt(view, now), dur = c.duration || 0, f = clamp(pos / (dur || 1));
    var pressed = V.pressed(view), loading = view.activity === 'loading', moving = view.activity === 'playing';
    var status = idle ? 'idle' : view.status, words = idle ? '' : view.words, rx = Math.round(tr.received * 100);
    var vol = view.muted ? 0 : view.volume, bufTo = (tr.buffered || 0) * dur;

    if (scope.documentElement) { setAttr(scope.documentElement, 'data-playing', pressed); setAttr(scope.documentElement, 'data-player', status); }
    find(HOSTS).forEach(function (el) {
      setAttr(el, 'data-status', status); setAttr(el, 'data-status-text', words); setVar(el, '--status-text', V.cssString(words));
      setAttr(el, 'data-playing', pressed); setVar(el, '--rx', rx);
    });
    find(LIKED_AT).forEach(function (el) { setAttr(el, 'data-liked', view.liked); setAttr(el, 'data-saved', view.saved); });
    find(SONG_POS_AT).forEach(function (el) { setVar(el, '--song-pos', Math.floor(f * 100) / 100); });
    find('[data-dk-status]').forEach(function (el) { setText(el, words); });

    /* the seek line and the wave: where the song is, what has arrived, and a seek on its way */
    var valText = V.time(pos) + ' of ' + (c.known ? '' : 'about ') + V.time(dur) + (tr.received < 1 ? ', downloaded to ' + V.time(bufTo) : '')
      + (view.seekWait != null ? ', waiting for ' + V.time(view.seekWait * dur) : '');
    var parts = tr.parts, bk = parts.map(function (r) { return r[0].toFixed(3) + '-' + r[1].toFixed(3); }).join('|');
    find(SEEKS).forEach(function (el) {
      setVar(el, '--pos', f.toFixed(4)); setVar(el, '--xfer', tr.front.toFixed(4));
      setClass(el, 'is-going', view.seekWait != null); if (view.seekWait != null) setVar(el, '--target', view.seekWait.toFixed(4));
      setClass(el, 'is-transferring', tr.transferring);
      var own = el.querySelector('[data-dk-buf]');
      if (own) {
        /* the mini player's own part (data-dk-buf): the arrived part the playhead is in, which a seek ahead may leave a gap before */
        var at = parts.filter(function (r) { return f >= r[0] && f <= r[1]; })[0] || [0, 0];
        setVar(own, 'left', (at[0] * 100) + '%'); setVar(own, 'width', ((at[1] - at[0]) * 100) + '%');
      } else if (el._bk !== bk) {
        el._bk = bk;
        var host = el.querySelector('.seek__bufs');
        if (host) host.innerHTML = parts.map(function (r) { return '<i class="seek__buf" style="left:' + (r[0] * 100).toFixed(2) + '%;width:' + ((r[1] - r[0]) * 100).toFixed(2) + '%"></i>'; }).join('');
      }
      setAttr(el, 'aria-valuenow', Math.round(f * 100)); setAttr(el, 'aria-valuetext', valText);
    });
    find(WAVES).forEach(function (el) {
      var bars = V.find(el, '.wave__bar'), n = bars.length;
      setVar(el, '--pos', f.toFixed(4)); setClass(el, 'is-playing', moving);
      bars.forEach(function (b, i) {
        var x = (i + .5) / n, inBuf = parts.some(function (r) { return x >= r[0] && x <= r[1]; });
        var cls = x <= f ? 'is-played' : inBuf ? 'is-buf' : x <= tr.front ? 'is-xfer' : '';
        if (b._c !== cls) { if (b._c) b.classList.remove(b._c); if (cls) b.classList.add(cls); b._c = cls; }
      });
      setAttr(el, 'aria-valuenow', Math.round(f * 100)); setAttr(el, 'aria-valuetext', valText);
    });
    find('.seek-row').forEach(function (el) { setVar(el, '--time-ch', V.time(dur, !c.known).length); });
    find('[data-bind="pos"], [data-dk-pos]').forEach(function (el) { setText(el, V.time(pos)); });
    find('[data-bind="dur"], [data-dk-dur]').forEach(function (el) { setText(el, V.time(dur, !c.known)); });
    find('[data-bind="buffered-to"]').forEach(function (el) { setText(el, V.time(bufTo)); });
    find('[data-bind="received"]').forEach(function (el) { setText(el, rx + '%'); });
    find('[data-dk-vol]').forEach(function (el) { setText(el, 'vol ' + Math.round(vol * 100) + '%'); });
    find('.seek[data-seek="volume"]').forEach(function (el) {
      setVar(el, '--pos', vol.toFixed(3)); setVar(el, '--xfer', 0);
      setAttr(el, 'aria-valuenow', Math.round(vol * 100)); setAttr(el, 'aria-valuetext', view.muted ? 'Muted' : Math.round(vol * 100) + '%');
    });

    /* the keys: play, like, save, shuffle and repeat, the page's (data-bind) and the mini player's (data-dk) alike. Their markup
       carries aria-keyshortcuts (a shortcut is the key's own attribute, never words in its name); a key's own words stay its own */
    function playKey(b, shortcut) {
      var label = pressed ? 'Pause' : 'Play';
      setAttr(b, 'aria-label', label); setAttr(b, 'data-tip', label + shortcut);
      keyIcon(b, pressed ? 'pause' : 'play', pressed ? '' : 'ic-play', true);
      setClass(b, 'is-buffering', loading);
    }
    find('[data-bind="play"]').forEach(function (b) { playKey(b, ' (Space)'); });
    find('[data-dk="play-pause"]').forEach(function (b) { playKey(b, ''); });
    find('[data-dk-play-label], [data-now="a-label"]').forEach(function (el) { setText(el, pressed ? 'pause' : 'play'); });
    var heart = view.liked ? 'heart-fill' : 'heart';
    find('[data-bind="like"]').forEach(function (b) {
      setAttr(b, 'data-id', t ? t.id : ''); setAttr(b, 'aria-pressed', view.liked); setClass(b, 'is-liked', view.liked); keyIcon(b, heart, null, true);
    });
    find('[data-dk="like"]').forEach(function (b) {
      keyIcon(b, heart, null, true); setAttr(b, 'aria-pressed', view.liked); setClass(b, 'is-liked', view.liked); setAttr(b, 'data-tip', view.liked ? 'Liked' : 'Like');
    });
    /* the page's Save key also shows the save's own steps and failure (app.js: its look); whether the song is saved is the view's */
    find('[data-bind="save"]').forEach(function (b) { setAttr(b, 'aria-pressed', view.saved); if (b.disabled !== !t) b.disabled = !t; });
    find('[data-dk="save"]').forEach(function (b) {
      setAttr(b, 'aria-pressed', view.saved); setClass(b, 'is-saved', view.saved); setAttr(b, 'data-tip', view.saved ? 'In your library' : 'Save to library');
    });
    find(SHUFFLE_KEYS).forEach(function (b) { setAttr(b, 'aria-pressed', view.shuffle); });
    find(REPEAT_KEYS).forEach(function (b) {
      setAttr(b, 'aria-pressed', view.repeat !== 'off');
      keyIcon(b, view.repeat === 'one' ? 'repeat-one' : 'repeat');
      if (b.hasAttribute('data-bind') || b.hasAttribute('data-act')) setAttr(b, 'aria-label', REPEAT_LABEL[view.repeat]);
      if (b.hasAttribute('data-bind')) setAttr(b, 'data-tip', REPEAT_LABEL[view.repeat] + ' (R)');
    });
    find('[data-now="repeat"]').forEach(function (el) { setText(el, view.repeat === 'one' ? 'repeat 1' : 'repeat'); });
  };

  root.PlayerView = V;
  if (typeof module === 'object' && module.exports) module.exports = V;
})(typeof self !== 'undefined' ? self : this);
