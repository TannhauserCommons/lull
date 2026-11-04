// The signals tests/align.json's cases are made of, as app/src/desktop/classic_native/align.rs makes them too: a
// seeded random walk (mulberry32, smoothed) long enough for every case, the song a stretch of it and the copy the same
// stretch `delay` seconds later, at `gain`, with `noise` of another walk added; `other` makes the copy another walk.
export function mulberry32(seed) {
  return () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
export function walk(seed, n) {
  const rnd = mulberry32(seed), x = new Float64Array(n);
  let y = 0;
  for (let i = 0; i < n; i++) { y = 0.95 * y + 0.05 * (rnd() * 2 - 1); x[i] = y * 4; }
  return x;
}
export function recordings(c) {
  const n = Math.round(c.seconds * c.rate), d = Math.round(c.delay * c.rate), pad = Math.round(2 * c.rate);
  const song = walk(c.seed, n + 2 * pad), other = walk(c.seed + 1, n + 2 * pad);
  const a = new Float32Array(n), b = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    a[i] = song[pad + i];
    b[i] = c.gain * (c.other ? other[pad + i] : song[pad + i - d]) + c.noise * other[pad + i];
  }
  return [a, b];
}

/* run directly (node tests/web/align-cases.mjs), it measures every case with web/align.js and writes what it measured
   into tests/align.json as `expect`: for a change to align.js that is meant, after which align.rs must match it */
import { createRequire } from 'node:module';
import { readFileSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const A = createRequire(import.meta.url)('../../web/align.js');
  const file = new URL('../align.json', import.meta.url), json = JSON.parse(readFileSync(file, 'utf8'));
  for (const c of json.cases) {
    const [a, b] = recordings(c), m = A.measure(a, b, c.rate, 1);
    c.expect = { lag: m.lag, confidence: m.confidence, margin: m.margin, quiet: m.quiet, similar: A.similar(m), locked: A.locked(m), levels: [A.rms(a), A.rms(b)] };
  }
  writeFileSync(file, JSON.stringify(json, null, 1) + '\n');
}
