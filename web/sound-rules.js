/* The sound rules (ADR-0081, docs/themes.md section 13): whether a sound may play, which pack it comes from, how a
   sound is levelled, and the device's sound settings. Pure functions over plain values, so node can test them
   (tests/web/sound-rules.test.mjs); web/sounds.js follows them. What a pack may declare is web/packs/vocabulary.json. */
(function (root) {
  'use strict';
  var S = {};

  /* timings and levels */
  S.REPEAT_MS = 80;            // the same sound again this soon is dropped
  S.PRESS_MS = 1000;           // a change this soon after the listener's tap or key is the listener's
  S.FADE_OUT_S = 0.06;         // the buffering loop fades out this fast once the music sounds
  S.FADE_IN_S = 0.3;
  S.SLEEP_MS = 10000;          // the sound engine's audio context sleeps after this long without a sound
  S.BOOT_WITHIN_MS = 4000;     // a start-up sound not ready this soon after the start is dropped, never played late
  S.PEAK = Math.pow(10, -12 / 20);   // every sound's peak at most -12 dBFS
  S.RMS = Math.pow(10, -24 / 20);    // and its loudness at most this, so no pack is louder than another
  S.MY_SOUNDS = 'my-sounds';

  /* ---------- the device's settings (localStorage 'lull:sounds') ---------- */
  S.DEFAULTS = {
    on: true,                // the master switch
    volume: 40,              // 0-100, a share of the device's volume
    onlySilent: true,        // only when no music is playing (play and pause still sound, at the press)
    followQuiet: true,       // quiet while the system asks for reduced motion or quiet
    surprises: true,         // the easter eggs
    sounds: {},              // event -> false (switched off) or true (on, for one off by default)
    background: {},          // event -> true: also while lull is in the background (vocabulary.background only)
    packs: [],               // pack ids the listener added, top first
    packsOff: {},            // pack id -> true: switched off
    eggsOff: {},             // egg name -> true: switched off
    replace: {}              // event -> { pack, sound }: that pack's sound for that event or egg
  };
  /* stored settings, whatever is stored: unknown keys dropped, wrong types back to their default */
  S.settings = function (raw) {
    var r = raw && typeof raw === 'object' ? raw : {}, out = {};
    Object.keys(S.DEFAULTS).forEach(function (k) {
      var d = S.DEFAULTS[k], v = r[k];
      if (Array.isArray(d)) out[k] = Array.isArray(v) ? v.filter(function (x) { return typeof x === 'string'; }) : [];
      else if (typeof d === 'object') out[k] = v && typeof v === 'object' && !Array.isArray(v) ? Object.assign({}, v) : {};
      else if (typeof d === 'number') out[k] = typeof v === 'number' && isFinite(v) ? Math.max(0, Math.min(100, Math.round(v))) : d;
      else out[k] = typeof v === 'boolean' ? v : d;
    });
    return out;
  };

  /* is this sound switched on? (a sound the vocabulary lists as off by default needs the listener's yes) */
  S.enabled = function (event, settings, vocab) {
    var v = settings.sounds[event];
    if (typeof v === 'boolean') return v;
    return !(vocab && vocab.offByDefault && vocab.offByDefault.indexOf(event) >= 0);
  };

  /* May `event` sound now? ctx:
       heard        music is coming out
       front        lull is the window in front (the page visible, one of lull's windows focused)
       quiet        the system asks for quiet (Android silent or vibrate, Windows' No Sounds, GNOME's event sounds off)
       reducedMotion  lull's Motion setting, else the system's
       classic      Classic mode is open
     An egg's sound passes `egg: true`: it follows the same rules, with the switches of Hidden surprises. */
  S.allowed = function (event, ctx, settings, vocab) {
    if (!settings.on) return false;
    if (ctx.classic && event !== 'classic') return false;
    if (ctx.egg ? !settings.surprises || settings.eggsOff[event] : !S.enabled(event, settings, vocab)) return false;
    if (settings.followQuiet && (ctx.quiet || ctx.reducedMotion)) return false;
    if (!ctx.front && !(!ctx.egg && settings.background[event] && vocab && vocab.background.indexOf(event) >= 0)) return false;
    if (S.isLoop(event, vocab)) return !ctx.heard;
    if (ctx.heard && settings.onlySilent && event !== 'play' && event !== 'pause') return false;
    return true;
  };
  S.isLoop = function (event, vocab) { return !!(vocab && vocab.loops.indexOf(event) >= 0); };

  /* ---------- packs ---------- */
  /* The packs that are on, top first (ADR-0081, "Which packs are on"): the listener's added packs in their order, the
     look's pack, then `default`; each once, minus those switched off. `look` is the look's pack id, or null.
     `known`: id -> pack, the packs this device has read. */
  S.stack = function (settings, look, known) {
    var ids = [], seen = {};
    settings.packs.concat(look ? [look] : [], ['default']).forEach(function (id) {
      if (seen[id] || settings.packsOff[id] || !known[id]) return;
      seen[id] = true; ids.push(id);
    });
    return ids.map(function (id) { return known[id]; });
  };
  /* the look's pack: the theme's own when it carries one, else the built-in pack of the design it is or extends
     (`builtIn`: the built-in packs' ids, web/packs/index.json) */
  S.lookPack = function (themeId, design, themesWithPack, builtIn) {
    if (themeId && themesWithPack.indexOf(themeId) >= 0) return 'theme:' + themeId;
    return design && builtIn.indexOf(design) >= 0 ? design : null;
  };

  /* the sound for an event or an egg: { pack, entry } where entry is { urls, loop }, or null.
     The listener's replacement first, then the first pack in the stack that has it; `null` in a pack silences it. */
  S.soundFor = function (name, stack, settings, known, egg) {
    var r = settings.replace[name];
    if (r && known[r.pack]) {
      var e = entryOf(known[r.pack], r.sound);
      if (e) return { pack: r.pack, entry: e };
    }
    for (var i = 0; i < stack.length; i++) {
      var p = stack[i];
      if (egg) {
        var g = S.eggs([p], settings)[name];
        if (g) return g.sound ? { pack: p.id, entry: normal(g.sound) } : null;
        continue;
      }
      if (!p.sounds || !Object.prototype.hasOwnProperty.call(p.sounds, name)) continue;
      return p.sounds[name] === null ? null : { pack: p.id, entry: normal(p.sounds[name]) };
    }
    return null;
  };
  function entryOf(pack, name) {
    if (pack.sounds && pack.sounds[name]) return normal(pack.sounds[name]);
    var g = (pack.eggs || []).filter(function (x) { return x.name === name && x.sound; })[0];
    return g ? normal(g.sound) : null;
  }
  /* a pack's sound value as the page reads it: URLs, already resolved by whoever read the pack */
  function normal(v) {
    if (typeof v === 'string') return { urls: [v], loop: false };
    if (Array.isArray(v)) return { urls: v.slice(), loop: false };
    return { urls: [v.url || v.file], loop: !!v.loop };
  }
  S.normal = normal;

  /* every egg that is on: by name, the highest pack's declaration of a name winning, minus those switched off */
  S.eggs = function (stack, settings) {
    var out = {};
    for (var i = stack.length - 1; i >= 0; i--) {
      (stack[i].eggs || []).forEach(function (g) { out[g.name] = Object.assign({ pack: stack[i].id }, g); });
    }
    Object.keys(settings.eggsOff).forEach(function (n) { if (settings.eggsOff[n]) delete out[n]; });
    return out;
  };

  /* the next of a sound's files: a list takes turns, so a press repeated never sounds mechanical */
  S.pick = function (entry, turn) { return entry.urls[turn % entry.urls.length]; };

  /* ---------- level ---------- */
  /* the gain that brings a sound's samples to lull's level: its peak to at most S.PEAK and its loudness (RMS over its
     loudest 50 ms window) to at most S.RMS, never louder than it was made */
  S.levelGain = function (samples, rate) {
    var peak = 0, win = Math.max(1, Math.round(rate * 0.05)), sum = 0, loudest = 0, i;
    for (i = 0; i < samples.length; i++) {
      var v = samples[i], a = v < 0 ? -v : v;
      if (a > peak) peak = a;
      sum += v * v;
      if (i >= win) { var o = samples[i - win]; sum -= o * o; }
      if (i >= win - 1 && sum > loudest) loudest = sum;
    }
    if (samples.length < win) loudest = sum;
    var rms = Math.sqrt(Math.max(0, loudest) / Math.min(win, samples.length || 1));
    if (!peak) return 0;
    return Math.min(1, S.PEAK / peak, rms ? S.RMS / rms : 1);
  };

  root.LullSoundRules = S;
  if (typeof module === 'object' && module.exports) module.exports = S;
})(typeof self !== 'undefined' ? self : this);
