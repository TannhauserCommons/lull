/* The equaliser, one per device (docs/context/web.md#equaliser): what lull's player and Classic's Webamp both play
   through. Its bands are Webamp's own (a low shelf at 60 Hz, peaks at 170 Hz to 14 kHz, a high shelf at 16 kHz,
   ±12 dB), so Webamp's equaliser window and Settings > Playback show and change the same thing.
     settings = { on, bands: [10 dB], preamp: dB | null }   null: the level is automatic (below)
   The simple face moves shapes over the bands: Bass and Treble, and named presets. The level before the filters is
   automatic unless set by hand (Webamp's preamp): the curve's highest point is taken back down to 0 dB, so a boost
   never clips. Pure math and storage here (tests/web/eq.test.mjs); chain(ctx) builds the Web Audio filters.
     read() / write(settings)   localStorage KEY; write tells this window's listeners, `storage` tells the others
     on(fn) -> off               fn(settings) after any change, from this window or another
     bass(s) / treble(s)         the Bass and Treble the bands show (dB)
     withBass(s, dB) / withTreble(s, dB) / withPreset(s, id) / withBand(s, i, dB) / withPreamp(s, dB|null)
     preset(s)                   the preset the bands match, or null (custom)
     level(s)                    dB before the filters: the hand-set preamp, or the automatic one
     response(s, hz)             the curve's gain at those frequencies (dB), as Web Audio's filters draw it
     active(s)                   anything to do: on, and not flat at 0 dB
     spec(s)                     what an output plays through (docs/context/web.md#output-interface): null when not
                                 active, else { levelDb, bands: [{ type: lowshelf|peaking|highshelf, hz, gainDb }] }
                                 (peaks have Q 1, shelves slope 1: Web Audio's defaults)
     chain(ctx) -> { input, output, set(spec) }   null: input goes straight to output (no filter runs) */
(function (root) {
  'use strict';
  var KEY = 'lull:eq';
  var BANDS = [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000];
  var MAX_DB = 12;
  var RATE = 48000;                                         // close enough for every rate: only the top band moves
  var MARGIN_DB = .1;                                       // the automatic level stays this far under 0 dB
  /* how much of a Bass or Treble move each band takes, so +6 dB of Bass peaks at +6 dB: a shelf, as a hi-fi's tone
     controls (the top bands overlap, so each takes less) */
  var BASS_W = [1, .9, .2, .05, 0, 0, 0, 0, 0, 0];
  var TREBLE_W = [0, 0, 0, 0, 0, .3, .8, .5, .15, .75];
  var PRESETS = [
    { id: 'flat', name: 'Flat', bands: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
    { id: 'bass', name: 'Bass boost', bands: shape(6, 0) },
    { id: 'treble', name: 'Treble boost', bands: shape(0, 6) },
    { id: 'both', name: 'Bass and treble', bands: shape(5, 4) },
    { id: 'voices', name: 'Voices', bands: [-2, -1.5, 0, 1.5, 3, 3.5, 2, 0, 0, 0] },
    { id: 'less-bass', name: 'Less bass', bands: shape(-6, 0) }
  ];
  function shape(bass, treble) { return BANDS.map(function (_, i) { return round(BASS_W[i] * bass + TREBLE_W[i] * treble); }); }

  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }
  function round(v) { return Math.round(v * 10) / 10 || 0; }      // || 0: never -0
  function db(v) { return round(clamp(+v || 0, -MAX_DB, MAX_DB)); }
  function flat() { return { on: true, bands: BANDS.map(function () { return 0; }), preamp: null }; }

  /* anything stored that isn't a whole equaliser reads as the flat one */
  function normalise(x) {
    if (!x || typeof x !== 'object' || !Array.isArray(x.bands) || x.bands.length !== BANDS.length) return flat();
    return { on: x.on !== false, bands: x.bands.map(db), preamp: x.preamp == null ? null : db(x.preamp) };
  }
  function read() {
    try { return normalise(JSON.parse(root.localStorage.getItem(KEY))); } catch (e) { return flat(); }
  }
  var subs = [];
  function write(s) {
    s = normalise(s);
    try { root.localStorage.setItem(KEY, JSON.stringify(s)); } catch (e) { /* storage blocked: this visit only */ }
    subs.slice().forEach(function (fn) { fn(s); });
    return s;
  }
  function on(fn) {
    subs.push(fn);
    return function () { subs = subs.filter(function (f) { return f !== fn; }); };
  }
  if (root.addEventListener) root.addEventListener('storage', function (e) {
    if (e.key !== KEY) return;
    var s = read(); subs.slice().forEach(function (fn) { fn(s); });
  });

  function same(a, b) { return a.length === b.length && a.every(function (v, i) { return Math.abs(v - b[i]) < .05; }); }
  /* a weighted mean: exact for a Bass or Treble shape, a fair reading of a curve drawn band by band */
  function along(s, w) {
    var num = 0, den = 0;
    for (var i = 0; i < w.length; i++) { num += w[i] * s.bands[i]; den += w[i] * w[i]; }
    return round(num / den);
  }
  /* moving Bass or Treble shifts its bands by its shape and keeps the rest of the curve; the level is automatic again */
  function shifted(s, w, to) {
    var by = db(to) - along(s, w);
    return { on: true, bands: s.bands.map(function (v, i) { return db(v + w[i] * by); }), preamp: null };
  }
  function presetOf(id) { return PRESETS.filter(function (p) { return p.id === id; })[0]; }
  function preset(s) { var p = PRESETS.filter(function (p) { return same(p.bands, s.bands); })[0]; return p ? p.id : null; }
  function withPreset(s, id) {
    var p = presetOf(id); if (!p) throw new Error('Unknown equaliser preset ' + id);
    return { on: true, bands: p.bands.slice(), preamp: null };
  }
  function withBand(s, i, v) { var b = s.bands.slice(); b[i] = db(v); return { on: s.on, bands: b, preamp: s.preamp }; }
  function withPreamp(s, v) { return { on: s.on, bands: s.bands.slice(), preamp: v == null ? null : db(v) }; }

  /* ---------- the curve, as Web Audio's BiquadFilterNode computes it (its spec's formulas, Q 1, shelf slope 1) ---------- */
  function coefficients(type, f0, gain) {
    var A = Math.pow(10, gain / 40), w0 = 2 * Math.PI * f0 / RATE, c = Math.cos(w0), sn = Math.sin(w0);
    if (type === 'peaking') {
      var al = sn / 2;
      return [1 + al * A, -2 * c, 1 - al * A, 1 + al / A, -2 * c, 1 - al / A];
    }
    var as = sn / 2 * Math.SQRT2, r = 2 * Math.sqrt(A) * as;
    if (type === 'lowshelf') {
      return [A * ((A + 1) - (A - 1) * c + r), 2 * A * ((A - 1) - (A + 1) * c), A * ((A + 1) - (A - 1) * c - r),
        (A + 1) + (A - 1) * c + r, -2 * ((A - 1) + (A + 1) * c), (A + 1) + (A - 1) * c - r];
    }
    return [A * ((A + 1) + (A - 1) * c + r), -2 * A * ((A - 1) + (A + 1) * c), A * ((A + 1) + (A - 1) * c - r),
      (A + 1) - (A - 1) * c + r, 2 * ((A - 1) - (A + 1) * c), (A + 1) - (A - 1) * c - r];
  }
  function typeOf(i) { return i === 0 ? 'lowshelf' : i === BANDS.length - 1 ? 'highshelf' : 'peaking'; }
  function magnitude(k, hz) {
    var w = 2 * Math.PI * hz / RATE, c1 = Math.cos(w), s1 = Math.sin(w), c2 = Math.cos(2 * w), s2 = Math.sin(2 * w);
    var br = k[0] + k[1] * c1 + k[2] * c2, bi = -(k[1] * s1 + k[2] * s2);
    var ar = k[3] + k[4] * c1 + k[5] * c2, ai = -(k[4] * s1 + k[5] * s2);
    return Math.sqrt((br * br + bi * bi) / (ar * ar + ai * ai));
  }
  function filters(s) { return s.bands.map(function (g, i) { return coefficients(typeOf(i), BANDS[i], g); }); }
  function response(s, hz) {
    var ks = filters(s);
    return hz.map(function (f) {
      var m = 1; for (var i = 0; i < ks.length; i++) m *= magnitude(ks[i], f);
      return 20 * Math.log10(m);
    });
  }
  /* 20 Hz to 20 kHz, a few points per third of an octave, and every band's own frequency */
  var GRID = (function () {
    var g = BANDS.slice(); for (var i = 0; i <= 120; i++) g.push(20 * Math.pow(1000, i / 120));
    return g.sort(function (a, b) { return a - b; });
  })();
  function autoLevel(s) {
    var peak = Math.max.apply(null, response(s, GRID));
    return peak > 0 ? -Math.round((peak + MARGIN_DB) * 100) / 100 : 0;
  }
  function level(s) { return s.preamp != null ? s.preamp : autoLevel(s); }
  function active(s) { return s.on && (s.bands.some(function (v) { return v !== 0; }) || s.preamp != null && s.preamp !== 0); }

  function spec(s) {
    if (!active(s)) return null;
    return { levelDb: level(s), bands: BANDS.map(function (hz, i) { return { type: typeOf(i), hz: hz, gainDb: s.bands[i] }; }) };
  }

  /* ---------- Web Audio: a level, then the ten filters, as Webamp's own chain ---------- */
  function chain(ctx) {
    var input = ctx.createGain(), output = ctx.createGain(), fs = BANDS.map(function (hz, i) {
      var f = ctx.createBiquadFilter(); f.type = typeOf(i); f.frequency.value = hz; f.gain.value = 0; return f;
    });
    fs.forEach(function (f, i) { f.connect(i + 1 < fs.length ? fs[i + 1] : output); });
    input.connect(output);
    var through = false;
    function set(sp) {
      var on = !!sp, t = ctx.currentTime;
      if (on !== through) { input.disconnect(); input.connect(on ? fs[0] : output); through = on; }
      input.gain.setTargetAtTime(on ? Math.pow(10, sp.levelDb / 20) : 1, t, .02);
      fs.forEach(function (f, i) { f.gain.setTargetAtTime(on ? sp.bands[i].gainDb : 0, t, .02); });
    }
    return { input: input, output: output, set: set };
  }

  var api = {
    KEY: KEY, BANDS: BANDS, MAX_DB: MAX_DB, PRESETS: PRESETS, flat: flat, normalise: normalise, read: read, write: write, on: on,
    bass: function (s) { return along(s, BASS_W); }, treble: function (s) { return along(s, TREBLE_W); },
    withBass: function (s, v) { return shifted(s, BASS_W, v); }, withTreble: function (s, v) { return shifted(s, TREBLE_W, v); },
    withPreset: withPreset, withBand: withBand, withPreamp: withPreamp, preset: preset, level: level, response: response,
    active: active, spec: spec, chain: chain
  };
  root.LullEq = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof self !== 'undefined' ? self : this);
