// The alignment math behind switching to a better copy mid-song (web/align.js): measured on synthetic recordings of
// one moment from two copies, delayed, re-mastered and noisy, with the residual offset reported per case.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const { measure, locked } = createRequire(import.meta.url)('../../web/align.js');
const RATE = 48000;

/* a deterministic random source (mulberry32) */
function random(seed) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* seconds of something music-like: a bass line, chords and drums on an uneven beat, over a quiet pad */
function song(seconds, seed) {
  const rnd = random(seed), n = Math.round(seconds * RATE), x = new Float64Array(n);
  const scale = [0, 2, 3, 5, 7, 8, 10];
  for (let at = 0; at < n; at += Math.round(RATE * (0.18 + rnd() * 0.3))) {
    const root = 110 * 2 ** (scale[Math.floor(rnd() * 7)] / 12), hit = rnd() < 0.5, len = Math.round(RATE * 0.6);
    for (let i = 0; i < len && at + i < n; i++) {
      const t = i / RATE, env = Math.exp(-t * 6);
      x[at + i] += env * (0.3 * Math.sin(2 * Math.PI * root / 2 * t) + 0.15 * Math.sin(2 * Math.PI * root * 1.5 * t) + 0.1 * Math.sin(2 * Math.PI * root * 2 * t));
      if (hit && i < RATE * 0.05) x[at + i] += 0.2 * Math.exp(-t * 60) * (rnd() * 2 - 1);
    }
  }
  for (let i = 0; i < n; i++) x[i] += 0.02 * Math.sin(2 * Math.PI * 220 * i / RATE);
  return x;
}

/* `from` seconds of x heard `delay` samples later (fractions interpolated) */
function heard(x, from, seconds, delay) {
  const n = Math.round(seconds * RATE), out = new Float32Array(n), start = from * RATE - delay;
  for (let i = 0; i < n; i++) {
    const p = start + i, j = Math.floor(p), f = p - j;
    out[i] = (x[j] || 0) * (1 - f) + (x[j + 1] || 0) * f;
  }
  return out;
}

/* another master or encode of the same audio: duller (a low-pass run both ways, so it moves nothing in time),
   quieter, a little hiss */
function remastered(x, seed) {
  const rnd = random(seed), out = new Float32Array(x.length), k = 0.35;
  let y = 0;
  for (let i = 0; i < x.length; i++) out[i] = y += k * (x[i] - y);
  y = 0;
  for (let i = x.length - 1; i >= 0; i--) out[i] = y += k * (out[i] - y);
  for (let i = 0; i < x.length; i++) out[i] = 0.7 * out[i] + 0.003 * (rnd() * 2 - 1);
  return out;
}

const music = song(8, 7);
const WINDOW = 3;

for (const [name, delay] of [['in step', 0], ['37 samples late', 37], ['a quarter second early', -12000], ['0.9 s late', 43200], ['a fraction of a sample late', 100.4]]) {
  test(`two copies ${name} line up within a sample`, (t) => {
    const a = heard(music, 2, WINDOW, 0);
    const b = remastered(heard(music, 2, WINDOW, delay), 3);
    const m = measure(a, b, RATE, 1);
    const residual = m.lag * RATE - delay;
    t.diagnostic(`residual ${residual.toFixed(3)} samples (${(residual / RATE * 1000).toFixed(4)} ms), confidence ${m.confidence.toFixed(3)}, margin ${m.margin.toFixed(3)}`);
    assert.ok(Math.abs(residual) <= 0.5, `residual ${residual} samples`);
    assert.ok(locked(m), JSON.stringify(m));
  });
}

test('a copy measured again after the correction reads as in step', (t) => {
  // The player delays the earlier deck by what it measured, then measures again: what is left is the residual.
  const a = heard(music, 4, WINDOW, 0), late = heard(music, 4, WINDOW, 913);
  const first = measure(a, remastered(late, 5), RATE, 1);
  const corrected = heard(music, 4, WINDOW, 913 - first.lag * RATE);
  const again = measure(a, remastered(corrected, 6), RATE, 0.05);
  t.diagnostic(`after correcting ${(first.lag * 1000).toFixed(3)} ms: residual ${(again.lag * 1000).toFixed(4)} ms`);
  assert.ok(Math.abs(again.lag) * 1000 < 0.02, `residual ${again.lag * 1000} ms`);
});

test('different songs never line up', () => {
  const m = measure(heard(music, 2, WINDOW, 0), heard(song(8, 99), 2, WINDOW, 0), RATE, 1);
  assert.ok(!locked(m), JSON.stringify(m));
});

test('silence says nothing', () => {
  const m = measure(new Float32Array(RATE * WINDOW), heard(music, 2, WINDOW, 0), RATE, 1);
  assert.ok(m.quiet && !locked(m));
});

test('a loop that repeats within the search is ambiguous, so it waits', () => {
  // Half a second played over and over: every multiple of it lines up as well as the true lag.
  const bar = music.subarray(RATE * 2, RATE * 2.5), loop = new Float64Array(RATE * 8);
  for (let i = 0; i < loop.length; i++) loop[i] = bar[i % bar.length];
  const m = measure(heard(loop, 2, WINDOW, 0), heard(loop, 2, WINDOW, 200), RATE, 1);
  assert.ok(m.margin < 0.1 && !locked(m), JSON.stringify(m));
});
