/* lull's UI sounds (ADR-0081, docs/themes.md section 13): which sound each moment makes, from the packs that are on,
   under the rules in sound-rules.js (never over the music, never while lull isn't in front, quiet when the system asks).
   It touches no DOM but the packs' stylesheet: a sound draws nothing. Files are fetched once the first screen has
   settled (the service worker keeps them), decoded once into Web Audio buffers and levelled as they decode; the audio
   context is lull's own, apart from the player's, starts on the first tap or key (autoplay rules) and sleeps after
   10 s without a sound. App.sounds is the API (Settings > Sounds, Theme Studio, eggs.js). */
(function () {
  'use strict';
  var App = window.App, S = window.LullSoundRules, P = App.player, ps = P.state, D = window.Designs;
  var root = document.documentElement;

  /* ---------- settings: per device ---------- */
  var settings = S.settings(App.storage('sounds'));
  function save() { App.storage('sounds', settings); App.emit('sounds', 'settings'); }

  /* ---------- what the page knows ---------- */
  var vocab = null;              // web/packs/vocabulary.json
  var known = {};                // pack id -> pack (built-in, the listener's, `theme:<id>` for a theme's own)
  var themesWithPack = [];
  var builtIn = [];              // the built-in packs (web/packs/index.json)
  var lastInput = -Infinity, cuedInput = null, armed = false;
  var appFront = null;           // the desktop app: one of lull's windows has the focus (null: not reported)
  var systemQuiet = false;       // the system asks for quiet (the app reports it; a browser can't know)
  var classic = root.getAttribute('data-classic') === 'on';
  var lastStatus = ps.status, lastUid = uidNow(), lastIndex = ps.index, lastLen = ps.queue.length;

  function uidNow() { var q = ps.queue[ps.index]; return q ? q.uid : null; }
  function now() { return performance.now(); }
  /* the listener's own: a change this soon after their tap or key on this page, or a press in one of the app's
     windows (App.sounds.pressed). A change another device made, or the player moving on by itself, is not. */
  function byListener() { return now() - lastInput < S.PRESS_MS; }
  function front() {
    if (document.visibilityState !== 'visible') return false;
    return appFront != null ? appFront : document.hasFocus();
  }
  function ctxNow(egg) {
    return { heard: P.heard(), front: front(), quiet: systemQuiet, reducedMotion: App.reducedMotion(), classic: classic, egg: !!egg };
  }
  function input() { lastInput = now(); armed = true; unlock(); }
  ['pointerdown', 'keydown'].forEach(function (type) { window.addEventListener(type, input, { capture: true, passive: true }); });

  /* ---------- the packs ---------- */
  function lookPack() {
    var m = D.list[D.current];
    return S.lookPack(m && !m.builtIn ? D.current : null, D.designOf(D.current), themesWithPack, builtIn);
  }
  function stack() { return S.stack(settings, lookPack(), known); }
  function getJson(url) {
    return fetch(url, { credentials: 'same-origin' }).then(function (r) {
      if (!r.ok) { var e = new Error(url + ': ' + r.status); e.status = r.status; throw e; }
      return r.json();
    });
  }
  /* a pack as the page uses it: every sound as a URL */
  function absorb(id, pack, base) {
    var abs = function (v) {
      if (v == null) return v;
      if (typeof v === 'string') return new URL(v, base).href;
      if (Array.isArray(v)) return v.map(abs);
      return { url: new URL(v.url || v.file, base).href, loop: !!v.loop };
    };
    var sounds = {};
    Object.keys(pack.sounds || {}).forEach(function (k) { sounds[k] = abs(pack.sounds[k]); });
    known[id] = {
      id: id, name: pack.name, author: pack.author || '', description: pack.description || '', source: pack.source || 'built-in',
      theme: pack.theme || null, css: pack.css ? new URL(pack.css, base).href : null, slots: pack.slots || [], sounds: sounds,
      eggs: (pack.eggs || []).map(function (g) { return g.sound ? Object.assign({}, g, { sound: abs(g.sound) }) : g; })
    };
  }
  var packsBase = new URL('packs/', document.baseURI).href, builtInCss = {};
  var loading = null;
  /* the vocabulary, the built-in packs that can be on, and the hub's (the listener's and themes'); again on a change */
  function load() {
    var index = builtIn.length ? Promise.resolve() : getJson(packsBase + 'index.json').then(function (j) {
      builtIn = j.packs.map(function (p) { return p.id; });
      j.packs.forEach(function (p) { if (p.css) builtInCss[p.id] = true; });
    });
    loading = Promise.all([
      vocab ? Promise.resolve(vocab) : getJson(packsBase + 'vocabulary.json'),
      index.then(function () {
        return Promise.all(builtIn.filter(wanted).map(function (id) {
          if (known[id]) return null;
          return getJson(packsBase + id + '/pack.json').then(function (p) {
            if (builtInCss[id]) p.css = 'pack.css';
            absorb(id, p, packsBase + id + '/');
          });
        }));
      }),
      App.data.api('/api/packs').then(function (r) {
        Object.keys(known).forEach(function (id) { if (known[id].source !== 'built-in') delete known[id]; });
        themesWithPack = [];
        (r.packs || []).forEach(function (p) {
          var id = p.source === 'theme' ? 'theme:' + p.theme : p.id;
          if (p.source === 'theme') themesWithPack.push(p.theme);
          absorb(id, p, location.href);
        });
      }, function (err) {
        // an older hub has none (404); a device not paired yet can't ask (401, 403)
        if ([401, 403, 404].indexOf(err.status) < 0) console.warn('The listener\'s sound packs didn\'t load', err);
      })
    ]).then(function (r) {
      vocab = r[0];
      applyCss(); App.emit('sounds', 'packs'); updateLoop(); warm();
    }, function (err) { console.warn('lull\'s sounds didn\'t load', err); loading = null; });
    return loading;
  }
  function wanted(id) { return id === 'default' || id === D.designOf(D.current) || settings.packs.indexOf(id) >= 0; }

  /* the stylesheets of the packs that are on (their surprises' looks), after the theme's, in the user layer */
  var packCss = null;
  function applyCss() {
    var css = stack().filter(function (p) { return p.css; }).reverse()
      .map(function (p) { return '@import url("' + p.css + '") layer(user);'; }).join('\n');
    if (!css) { if (packCss) { packCss.remove(); packCss = null; } return; }
    if (!packCss) {
      packCss = document.createElement('style'); packCss.id = 'pack-css';
      var user = document.getElementById('user-css');
      if (user && user.parentNode) user.parentNode.insertBefore(packCss, user.nextSibling); else document.head.appendChild(packCss);
    }
    if (packCss.textContent !== css) packCss.textContent = css;
  }

  /* ---------- audio ---------- */
  var ctx = null, master = null, sleepTimer = 0;
  /* the context: made on the first gesture, or at the start where the platform lets audio start by itself (the app,
     an installed PWA), so a browser tab never warns about a context it may not start */
  function context(atStart) {
    if (ctx) return ctx;
    if (atStart && !selfStarting()) return null;
    var AC = window.AudioContext || window.webkitAudioContext;
    if (!AC) return null;
    ctx = new AC({ latencyHint: 'interactive' });
    master = ctx.createGain(); master.gain.value = volume(); master.connect(ctx.destination);
    return ctx;
  }
  function selfStarting() {
    return !!(window.__TAURI__ || window.LullNative || (window.matchMedia && matchMedia('(display-mode: standalone)').matches));
  }
  function unlock() { var c = context(false); if (c && c.state !== 'running') c.resume().then(null, function () {}); warm(); }
  function volume() { return Math.pow(settings.volume / 100, 2); }    // perceived volume: the slider's square
  function awake() {
    clearTimeout(sleepTimer);
    sleepTimer = setTimeout(function () { if (ctx && !loopSrc && ctx.state === 'running') ctx.suspend(); }, S.SLEEP_MS);
    if (ctx.state !== 'running') return ctx.resume();
    return Promise.resolve();
  }

  /* url -> promise of { buffer, gain }: fetched and decoded once, levelled once */
  var decoded = {};
  function buffer(url) {
    if (decoded[url]) return decoded[url];
    var c = context(false);
    if (!c) return Promise.reject(new Error('no audio context yet'));
    decoded[url] = fetch(url, { credentials: 'same-origin' }).then(function (r) {
      if (!r.ok) throw new Error(url + ': ' + r.status);
      return r.arrayBuffer();
    }).then(function (bytes) {
      return new Promise(function (ok, fail) { c.decodeAudioData(bytes, ok, fail); });
    }).then(function (b) {
      return { buffer: b, gain: S.levelGain(b.getChannelData(0), b.sampleRate) };
    });
    decoded[url].then(null, function (err) { console.warn('A sound didn\'t decode', url, err); delete decoded[url]; });
    return decoded[url];
  }
  /* decode the sounds that are on, a few at a time, once there is a context to decode them with */
  function warm() {
    if (!ctx || !vocab) return;
    var urls = [], st = stack();
    vocab.events.forEach(function (e) {
      if (!S.enabled(e, settings, vocab)) return;
      var s = S.soundFor(e, st, settings, known);
      if (s) urls = urls.concat(s.entry.urls);
    });
    urls.forEach(function (u) { buffer(u).then(null, function () {}); });
  }

  var turns = {}, current = null, lastPlayed = {};
  /* play a resolved sound now: one at a time, the same sound again within 80 ms dropped */
  function start(name, entry, opts) {
    var t = now();
    if (lastPlayed[name] && t - lastPlayed[name] < S.REPEAT_MS) return Promise.resolve(false);
    lastPlayed[name] = t;
    var url = S.pick(entry, turns[name] = (turns[name] || 0) + 1);
    return buffer(url).then(function (b) {
      if (opts && opts.still && !opts.still()) return false;      // the moment passed while it decoded
      return awake().then(function () {
        if (current) fadeStop(current, 0.015);
        var src = ctx.createBufferSource(), g = ctx.createGain();
        src.buffer = b.buffer; g.gain.value = b.gain;
        src.connect(g); g.connect(master);
        src.start();
        current = { src: src, gain: g };
        src.onended = function () { if (current && current.src === src) current = null; };
        App.emit('sound', name);
        return true;
      });
    }, function () { return false; });
  }
  function fadeStop(node, sec) {
    var t = ctx.currentTime;
    node.gain.gain.cancelScheduledValues(t);
    node.gain.gain.setValueAtTime(node.gain.gain.value, t);
    node.gain.gain.linearRampToValueAtTime(0, t + sec);
    try { node.src.stop(t + sec + 0.01); } catch (e) { /* already stopped */ }
  }
  /* an event's sound, if the rules let it sound now; an egg that takes its place (eggs.js) plays instead */
  function cue(event) {
    App.emit('moment', event);               // an egg may answer the moment itself (eggs.js), sound or not
    if (!vocab || !ctx) return;
    if (!S.allowed(event, ctxNow(false), settings, vocab)) return;
    var instead = App.eggs && App.eggs.instead(event);
    if (instead) { App.sounds.egg(instead); return; }
    var s = S.soundFor(event, stack(), settings, known);
    if (s) start(event, s.entry, { still: function () { return S.allowed(event, ctxNow(false), settings, vocab); } });
  }
  /* a press's sound: one per tap or key, the first thing it did */
  function press(event) {
    if (!byListener() || cuedInput === lastInput) return;
    cuedInput = lastInput;
    cue(event);
  }

  /* ---------- the buffering loop: only in silence, gone the moment the music sounds ---------- */
  var loopSrc = null, loopUrl = null, loopInstead = null;
  function waiting() {
    return armed && ps.playing && !P.heard() && (ps.status === 'finding' || ps.status === 'queued' || ps.status === 'buffering');
  }
  function loopEntry() {
    if (loopInstead) { var g = S.soundFor(loopInstead, stack(), settings, known, true); if (g && g.entry.loop) return g.entry; }
    var s = S.soundFor('buffering', stack(), settings, known);
    return s && s.entry.loop ? s.entry : null;
  }
  function updateLoop() {
    var want = vocab && ctx && waiting() && S.allowed('buffering', ctxNow(false), settings, vocab) ? loopEntry() : null;
    var url = want ? want.urls[0] : null;
    if (url === loopUrl) return;
    if (loopSrc) { fadeStop(loopSrc, loopUrl && url ? S.FADE_IN_S : S.FADE_OUT_S); loopSrc = null; }
    loopUrl = url;
    if (!url) return;
    buffer(url).then(function (b) {
      if (loopUrl !== url || loopSrc) return;
      return awake().then(function () {
        if (loopUrl !== url || loopSrc) return;
        var src = ctx.createBufferSource(), g = ctx.createGain(), t = ctx.currentTime;
        src.buffer = b.buffer; src.loop = true;
        g.gain.setValueAtTime(0, t); g.gain.linearRampToValueAtTime(b.gain, t + S.FADE_IN_S);
        src.connect(g); g.connect(master); src.start();
        loopSrc = { src: src, gain: g };
      });
    }, function () {});
  }
  /* the music's first sound: the loop goes at once, before anything else hears of it */
  App.on('heard', function () { updateLoop(); });

  /* ---------- the moments ---------- */
  App.on('player', function (kind) {
    var uid = uidNow(), st = ps.status;
    if (kind === 'play') press(ps.playing ? 'play' : 'pause');
    if (kind === 'seek') press('seek');
    if (kind === 'track' && uid !== lastUid) {
      var step = ps.queue.length === lastLen && Math.abs(ps.index - lastIndex) === 1;
      press(step ? 'skip' : 'play');
    }
    if (kind === 'queue' && ps.queue.length > lastLen && uid === lastUid) press('queued');
    if (armed && ps.playing && uid === lastUid && st !== lastStatus) {
      if ((lastStatus === 'finding' || lastStatus === 'queued' || lastStatus === 'choose') && (st === 'buffering' || st === 'streaming')) cue('found');
      else if (st === 'stalled') cue('stalled');
    }
    if (armed && uid === lastUid && st !== lastStatus && (st === 'failed' || st === 'jellyfin-error')) cue('error');
    lastStatus = st; lastUid = uid; lastIndex = ps.index; lastLen = ps.queue.length;
    updateLoop();
  });
  App.on('like', function (d) { if (d.liked) press('liked'); });
  App.on('toast', function (o) { if (o.kind === 'error' && byListener()) cue('error'); });
  App.on('design', function () { press('design'); load(); });
  App.on('classic', function (open) {
    if (open && !classic && byListener()) { classic = true; cue('classic'); }
    classic = !!open; updateLoop();
  });
  /* a save the listener started here sounds when it is done; another device's saves don't */
  var mySaves = {};
  App.on('save', function (d) {
    var s = d.save || {};
    if (byListener() && s.status !== 'saved') mySaves[d.id] = true;
    if (s.status === 'saved' && mySaves[d.id]) { delete mySaves[d.id]; cue('saved'); }
  });
  document.addEventListener('visibilitychange', updateLoop);
  window.addEventListener('blur', updateLoop); window.addEventListener('focus', updateLoop);
  App.on('server:pack.changed', function () { load(); });
  App.on('server:theme.changed', function () { load(); });

  /* the app's windows and the system's quiet (the desktop app and Android report them; a browser can't) */
  var bridge = window.LullDesktop || (window.LullNative && window.LullNative.sound) || null;
  if (bridge && bridge.onFront) bridge.onFront(function (v) { appFront = !!v; updateLoop(); });
  if (bridge && bridge.onQuiet) bridge.onQuiet(function (v) { systemQuiet = !!v; updateLoop(); });

  /* ---------- the start-up sound: once per app start, never on a reload, never late ---------- */
  var started = false;
  try { started = !!sessionStorage.getItem('lull:started'); sessionStorage.setItem('lull:started', '1'); } catch (e) { started = true; }
  var bootAt = now();
  App.on('ready', function () {
    var bootWanted = !started && settings.on && !!context(true);
    if (bootWanted) {
      // the eggs first, so one can take the start-up sound's place
      Promise.all([load(), settings.surprises ? App.needEggs().then(null, function () {}) : null]).then(function () {
        if (now() - bootAt > S.BOOT_WITHIN_MS) return;
        armed = true;
        if (ctx.state === 'running') cue('boot');
      });
    } else {
      // the rest wait until the first screen has settled and the browser is idle
      setTimeout(function () {
        var go = function () { load(); if (settings.surprises) App.needEggs().then(null, function () {}); };
        if (window.requestIdleCallback) requestIdleCallback(go, { timeout: 5000 }); else go();
      }, 1500);
    }
  });

  /* ---------- the API ---------- */
  /* the credits, which the `credits` egg opens: eggs.js draws them */
  App.route('/credits', { title: 'Credits', need: function () { return App.eggs ? null : App.needEggs(); }, render: function () { return ''; } });
  App.needEggs = function () {
    return App.eggs ? Promise.resolve() : App.needScript('egg-rules.js', 'the easter eggs').then(function () { return App.needScript('eggs.js', 'the easter eggs'); });
  };
  App.sounds = {
    settings: function () { return settings; },
    /* change settings: a patch of S.DEFAULTS' keys */
    set: function (patch) {
      settings = S.settings(Object.assign({}, settings, patch));
      if (master) master.gain.setTargetAtTime(volume(), ctx.currentTime, 0.02);
      save(); applyCss(); updateLoop();
      if ('packs' in patch || 'packsOff' in patch) load(); else warm();
      if (settings.surprises && !App.eggs) App.needEggs().then(null, function () {});
    },
    vocab: function () { return vocab; },
    load: function () { return loading || load(); },
    reload: load,
    known: function () { return known; },
    stack: stack,
    look: lookPack,
    /* what Settings plays to let the listener hear a sound: always, at the device's sound volume */
    hear: function (name, packId, egg) {
      var c = context(false); if (!c) return Promise.resolve(false);
      var s = packId && known[packId] ? S.soundFor(name, [known[packId]], S.settings({}), known, egg) : S.soundFor(name, stack(), settings, known, egg);
      return s ? start(name, s.entry) : Promise.resolve(false);
    },
    /* an egg's sound (eggs.js), under the same rules, with Hidden surprises' switches */
    egg: function (name) {
      if (!vocab || !ctx || !S.allowed(name, ctxNow(true), settings, vocab)) return Promise.resolve(false);
      var s = S.soundFor(name, stack(), settings, known, true);
      return s && !s.entry.loop ? start(name, s.entry) : Promise.resolve(false);
    },
    /* an egg whose looped sound takes the buffering loop's place while it is live (eggs.js), or null */
    loopInstead: function (name) { loopInstead = name; updateLoop(); },
    eggs: function () { return S.eggs(stack(), S.settings(Object.assign({}, settings, { eggsOff: {} }))); },
    /* a press in one of the app's own windows (the mini player): the listener's, as a tap here is */
    pressed: function () { lastInput = now(); armed = true; },
    front: front
  };
})();
