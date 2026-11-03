/* Lining two copies of the same song up (docs/context/web.md#better-copies): how far apart two recordings of the
   same moment are, measured from the audio itself. The player records both decks at once and corrects by the lag;
   these are pure functions over sample arrays, so tests/web/align.test.mjs measures them.
   measure(a, b, rate, maxLag) -> { lag, confidence, margin, quiet }:
     lag         seconds b sounds later than a (negative: earlier), to a fraction of a sample;
     confidence  normalised correlation of the two at that lag below ~1.5 kHz, 0-1 (the same master re-encoded: above
                 0.9). Judged on the low band: encoders and masters differ most in the highs, never in when a note is;
     margin      how far that beats the best other lag more than 30 ms away (a loop repeating within maxLag: ~0);
     quiet       too little sound in either to tell. */
(function (root) {
  'use strict';
  var COARSE_RATE = 3000;           // Hz the wide search runs at: averaging is its low-pass
  var GUARD = 0.03;                 // s around the best lag that belong to its peak (a bass note's period)
  var SILENT = 1e-4;                // RMS below which a recording says nothing
  var CONFIDENT = 0.6, CLEAR = 0.1; // what locked() asks of a measurement
  var ALIKE = 0.45;                 // what similar() asks: another master may share less above the bass

  function rms(x) {
    var s = 0;
    for (var i = 0; i < x.length; i++) s += x[i] * x[i];
    return x.length ? Math.sqrt(s / x.length) : 0;
  }

  /* box-averaged down by `f`, mean removed */
  function decimate(x, f) {
    var n = Math.floor(x.length / f), out = new Float64Array(n), mean = 0;
    for (var i = 0; i < n; i++) {
      var s = 0;
      for (var j = 0; j < f; j++) s += x[i * f + j];
      out[i] = s / f; mean += out[i];
    }
    mean /= n || 1;
    for (i = 0; i < n; i++) out[i] -= mean;
    return out;
  }

  function squares(x) {
    var p = new Float64Array(x.length + 1);
    for (var i = 0; i < x.length; i++) p[i + 1] = p[i] + x[i] * x[i];
    return p;
  }

  /* normalised correlation of b[n] with a[n - k] over their overlap; pa, pb are prefix sums of squares */
  function ncc(a, b, pa, pb, k) {
    var n = Math.min(a.length, b.length), from = Math.max(0, k), to = Math.min(n, n + k);
    if (to - from < 2) return 0;
    var s = 0;
    for (var i = from; i < to; i++) s += a[i - k] * b[i];
    var ea = pa[to - k] - pa[from - k], eb = pb[to] - pb[from];
    return ea > 0 && eb > 0 ? s / Math.sqrt(ea * eb) : 0;
  }

  function measure(a, b, rate, maxLag) {
    var none = { lag: 0, confidence: 0, margin: 0, quiet: true };
    var n = Math.min(a.length, b.length);
    if (!n || rms(a) < SILENT || rms(b) < SILENT) return none;
    /* wide: every lag within maxLag (at most half the recording, so every overlap is long enough to judge) */
    var f = Math.max(1, Math.round(rate / COARSE_RATE)), ca = decimate(a, f), cb = decimate(b, f);
    var K = Math.min(Math.round(maxLag * rate / f), Math.floor(ca.length / 2)), cpa = squares(ca), cpb = squares(cb);
    var c = new Float64Array(2 * K + 1), best = -K;
    for (var k = -K; k <= K; k++) {
      c[k + K] = ncc(ca, cb, cpa, cpb, k);
      if (c[k + K] > c[best + K]) best = k;
    }
    var guard = Math.ceil(GUARD * rate / f), other = 0;
    for (k = -K; k <= K; k++) if (Math.abs(k - best) > guard) other = Math.max(other, c[k + K]);
    /* fine: every sample around it at the full rate, then the parabola through the top three */
    var fa = Float64Array.from(a.subarray ? a.subarray(0, n) : a.slice(0, n)), fb = Float64Array.from(b.subarray ? b.subarray(0, n) : b.slice(0, n));
    var pa = squares(fa), pb = squares(fb), at = best * f, top = -Infinity, peak = at;
    var near = {};
    for (k = at - 2 * f; k <= at + 2 * f; k++) {
      near[k] = ncc(fa, fb, pa, pb, k);
      if (near[k] > top) { top = near[k]; peak = k; }
    }
    var l = near[peak - 1] != null ? near[peak - 1] : ncc(fa, fb, pa, pb, peak - 1);
    var r = near[peak + 1] != null ? near[peak + 1] : ncc(fa, fb, pa, pb, peak + 1);
    var bend = l - 2 * top + r, shift = bend < 0 ? clamp(.5 * (l - r) / bend, -.5, .5) : 0;
    return { lag: (peak + shift) / rate, confidence: Math.max(0, c[best + K]), margin: Math.max(0, c[best + K] - other), quiet: false };
  }

  function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }

  /* the copies sound alike at the lag measured: enough for a narrow check near the lag locked() found */
  function similar(m) { return !m.quiet && m.confidence >= ALIKE; }
  /* a measurement good enough to switch on: the copies clearly line up at one lag, and no other */
  function locked(m) { return similar(m) && m.margin >= CLEAR; }

  var api = { measure: measure, locked: locked, similar: similar, rms: rms };
  root.LullAlign = api;
  if (typeof module === 'object' && module.exports) module.exports = api;
})(typeof self !== 'undefined' ? self : this);
