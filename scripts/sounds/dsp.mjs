// A small, deterministic synthesiser for lull's built-in sound packs (ADR-0081).
// Everything is mono at 48 kHz, as Float32Array. Randomness comes from a seeded generator, so the same source
// always renders the same samples on every machine (render.mjs --check compares them).

export const RATE = 48000;
const TAU = Math.PI * 2;

/** Seconds to samples. */
export const n = (sec) => Math.max(1, Math.round(sec * RATE));

/** A seeded generator (mulberry32): `rng()` gives [0, 1). */
export function seeded(seed) {
  let a = seed >>> 0;
  return function rng() {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A buffer `sec` long, filled by `fn(t, i)` (t in seconds). */
export function render(sec, fn) {
  const len = n(sec), out = new Float32Array(len);
  for (let i = 0; i < len; i++) out[i] = fn(i / RATE, i);
  return out;
}

export const silence = (sec) => new Float32Array(n(sec));

/** Mixes `src` into `dst` at `at` seconds, times `gain`. Grows nothing: what falls past the end is dropped. */
export function mixInto(dst, src, at = 0, gain = 1) {
  const o = Math.round(at * RATE);
  for (let i = 0; i < src.length && o + i < dst.length; i++) if (o + i >= 0) dst[o + i] += src[i] * gain;
  return dst;
}

/** A buffer as long as needed for every `[buffer, at, gain]` part. */
export function mix(parts, tail = 0) {
  let len = 0;
  for (const [b, at = 0] of parts) len = Math.max(len, Math.round(at * RATE) + b.length);
  const out = new Float32Array(len + n(tail));
  for (const [b, at = 0, g = 1] of parts) mixInto(out, b, at, g);
  return out;
}

export function gain(buf, g) { for (let i = 0; i < buf.length; i++) buf[i] *= g; return buf; }

/** Multiplies by an envelope `fn(t)`. */
export function shape(buf, fn) { for (let i = 0; i < buf.length; i++) buf[i] *= fn(i / RATE); return buf; }

/** Attack, then exponential decay to -60 dB over `decay` seconds. */
export function perc(attack, decay) {
  return (t) => (t < attack ? t / attack : Math.exp(-6.9 * (t - attack) / decay));
}

/** Linear attack, hold, linear release over a total `len`. */
export function ahr(attack, release, len) {
  return (t) => (t < attack ? t / attack : t > len - release ? Math.max(0, (len - t) / release) : 1);
}

/** A soft rise and fall shaped like half a sine, over `len`. */
export const arch = (len) => (t) => (t >= len ? 0 : Math.sin(Math.PI * t / len));

/** A phase accumulator for a frequency that can change over time: `osc(freqFn, wave)`. */
export function osc(sec, freq, wave = sine, phase0 = 0) {
  let ph = phase0;
  return render(sec, (t) => {
    const f = typeof freq === 'function' ? freq(t) : freq;
    const v = wave(ph);
    ph += f / RATE;
    ph -= Math.floor(ph);
    return v;
  });
}

export const sine = (p) => Math.sin(TAU * p);
export const tri = (p) => (p < 0.5 ? 4 * p - 1 : 3 - 4 * p);
/** A square whose edges are softened by `soft` (0..0.5 of a cycle), so 8-bit tones don't alias harshly. */
export const square = (duty = 0.5, soft = 0.02) => (p) => {
  const edge = (x) => Math.max(-1, Math.min(1, x / soft));
  return p < duty ? edge(Math.min(p, duty - p)) : -edge(Math.min(p - duty, 1 - p));
};

/** White noise. */
export function noise(sec, rng) { return render(sec, () => rng() * 2 - 1); }

/** Pink-ish noise (Paul Kellet's filter). */
export function pink(sec, rng) {
  let b0 = 0, b1 = 0, b2 = 0, b3 = 0, b4 = 0, b5 = 0, b6 = 0;
  return render(sec, () => {
    const w = rng() * 2 - 1;
    b0 = 0.99886 * b0 + w * 0.0555179; b1 = 0.99332 * b1 + w * 0.0750759; b2 = 0.969 * b2 + w * 0.153852;
    b3 = 0.8665 * b3 + w * 0.3104856; b4 = 0.55 * b4 + w * 0.5329522; b5 = -0.7616 * b5 - w * 0.016898;
    const v = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362) * 0.11;
    b6 = w * 0.115926;
    return v;
  });
}

/** Brown noise (integrated white), for rumbles and wind. */
export function brown(sec, rng) {
  let last = 0;
  return render(sec, () => { last = (last + 0.02 * (rng() * 2 - 1)) / 1.02; return last * 3.5; });
}

/**
 * A biquad (RBJ's cookbook) run over `buf` in place. `freq` and `q` may be functions of time, for sweeps.
 * Types: lowpass, highpass, bandpass, peak (with `db`).
 */
export function biquad(buf, type, freq, q = 0.707, db = 0) {
  let x1 = 0, x2 = 0, y1 = 0, y2 = 0;
  let b0, b1, b2, a1, a2, lastF = -1, lastQ = -1;
  const coef = (f, qq) => {
    const w = TAU * Math.min(f, RATE * 0.49) / RATE, cw = Math.cos(w), sw = Math.sin(w), alpha = sw / (2 * qq);
    let B0, B1, B2, A0, A1, A2;
    if (type === 'lowpass') { B0 = (1 - cw) / 2; B1 = 1 - cw; B2 = (1 - cw) / 2; A0 = 1 + alpha; A1 = -2 * cw; A2 = 1 - alpha; }
    else if (type === 'highpass') { B0 = (1 + cw) / 2; B1 = -(1 + cw); B2 = (1 + cw) / 2; A0 = 1 + alpha; A1 = -2 * cw; A2 = 1 - alpha; }
    else if (type === 'bandpass') { B0 = alpha; B1 = 0; B2 = -alpha; A0 = 1 + alpha; A1 = -2 * cw; A2 = 1 - alpha; }
    else if (type === 'peak') {
      const A = Math.pow(10, db / 40);
      B0 = 1 + alpha * A; B1 = -2 * cw; B2 = 1 - alpha * A; A0 = 1 + alpha / A; A1 = -2 * cw; A2 = 1 - alpha / A;
    } else throw new Error('unknown filter ' + type);
    b0 = B0 / A0; b1 = B1 / A0; b2 = B2 / A0; a1 = A1 / A0; a2 = A2 / A0;
  };
  for (let i = 0; i < buf.length; i++) {
    const t = i / RATE;
    const f = typeof freq === 'function' ? freq(t) : freq, qq = typeof q === 'function' ? q(t) : q;
    if (f !== lastF || qq !== lastQ) { coef(f, qq); lastF = f; lastQ = qq; }
    const x = buf[i], y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
    x2 = x1; x1 = x; y2 = y1; y1 = y;
    buf[i] = y;
  }
  return buf;
}

/** A struck bar or bell: decaying partials at `ratios` of `freq`, each with its own level and decay. */
export function bell(sec, freq, partials, attack = 0.002) {
  const out = new Float32Array(n(sec));
  for (const [ratio, level, decay] of partials) {
    const b = shape(osc(sec, freq * ratio), perc(attack, decay));
    mixInto(out, b, 0, level);
  }
  return out;
}

/** Two-operator FM: a carrier at `freq` modulated at `freq * ratio`, the index falling with `indexEnv`. */
export function fm(sec, freq, ratio, index, indexEnv = () => 1, wave = sine) {
  let pc = 0, pm = 0;
  return render(sec, (t) => {
    const m = Math.sin(TAU * pm) * index * indexEnv(t);
    const v = wave(pc + m / TAU);
    pc += freq / RATE; pm += freq * ratio / RATE;
    pc -= Math.floor(pc); pm -= Math.floor(pm);
    return v;
  });
}

/** Karplus-Strong: a plucked string or a soft mallet, `bright` 0..1. */
export function pluck(sec, freq, rng, bright = 0.5, decay = 0.996) {
  const period = Math.max(2, Math.round(RATE / freq)), line = new Float32Array(period);
  for (let i = 0; i < period; i++) line[i] = rng() * 2 - 1;
  let idx = 0;
  return render(sec, () => {
    const a = line[idx], b = line[(idx + 1) % period];
    const v = decay * (bright * a + (1 - bright) * 0.5 * (a + b));
    line[idx] = v; idx = (idx + 1) % period;
    return a;
  });
}

/** A small, smooth room: Schroeder's four combs and two allpasses. `wet` 0..1. Lengthens by `tail` s. */
export function room(buf, size = 0.5, damp = 0.4, wet = 0.3, tail = 0.8) {
  const len = buf.length + n(tail), out = new Float32Array(len);
  const combs = [1557, 1617, 1491, 1422].map((d) => ({ d: Math.round(d * (0.6 + size)), buf: null, i: 0, lp: 0 }));
  combs.forEach((c) => { c.buf = new Float32Array(c.d); });
  const aps = [225, 556].map((d) => ({ d, buf: new Float32Array(d), i: 0 }));
  const fb = 0.7 + size * 0.25;
  for (let i = 0; i < len; i++) {
    const x = i < buf.length ? buf[i] : 0;
    let s = 0;
    for (const c of combs) {
      const y = c.buf[c.i];
      c.lp = y * (1 - damp) + c.lp * damp;
      c.buf[c.i] = x + c.lp * fb;
      c.i = (c.i + 1) % c.d;
      s += y;
    }
    s *= 0.25;
    for (const a of aps) {
      const y = a.buf[a.i], v = -s * 0.5 + y;
      a.buf[a.i] = s + y * 0.5;
      a.i = (a.i + 1) % a.d;
      s = v;
    }
    out[i] = x * (1 - wet) + s * wet;
  }
  return out;
}

/** A short echo: `times` repeats `delay` s apart, each `fall` of the last. */
export function echo(buf, delay, fall, times = 3) {
  const parts = [[buf, 0, 1]];
  for (let k = 1; k <= times; k++) parts.push([buf, delay * k, Math.pow(fall, k)]);
  return mix(parts);
}

/** Soft saturation, for warmth. */
export function warm(buf, drive = 1.5) {
  const k = Math.tanh(drive);
  for (let i = 0; i < buf.length; i++) buf[i] = Math.tanh(buf[i] * drive) / k;
  return buf;
}

/** Fades both ends a little so no sound starts or stops with a click. */
export function edges(buf, inSec = 0.002, outSec = 0.01) {
  const a = n(inSec), b = n(outSec);
  for (let i = 0; i < a && i < buf.length; i++) buf[i] *= i / a;
  for (let i = 0; i < b && i < buf.length; i++) buf[buf.length - 1 - i] *= i / b;
  return buf;
}

/** Drops the silent tail (below -50 dB of the peak, inaudible at a UI sound's level), keeping a short fade. */
export function trim(buf) {
  let peak = 0;
  for (const v of buf) peak = Math.max(peak, Math.abs(v));
  const floor = peak * 0.003;
  let end = buf.length;
  while (end > 1 && Math.abs(buf[end - 1]) < floor) end--;
  return edges(buf.slice(0, Math.min(buf.length, end + n(0.01))), 0.001, 0.008);
}

/**
 * Makes a loop seamless: the last `xfade` s are blended into the first, and the buffer is shortened by as much,
 * so playing it end to start never jumps.
 */
export function seamless(buf, xfade = 0.4) {
  const x = n(xfade), len = buf.length - x, out = buf.slice(0, len);
  for (let i = 0; i < x; i++) {
    const w = i / x, a = Math.sqrt(w), b = Math.sqrt(1 - w);
    out[i] = buf[i] * a + buf[len + i] * b;
  }
  return out;
}

/** Scales to a peak of `db` dBFS. */
export function peakTo(buf, db = -3) {
  let peak = 0;
  for (const v of buf) peak = Math.max(peak, Math.abs(v));
  if (peak > 0) gain(buf, Math.pow(10, db / 20) / peak);
  return buf;
}

/** Equal-tempered pitch: `hz('A4')`, `hz('C#5')`. */
export function hz(note) {
  const m = /^([A-G])(#|b)?(-?\d)$/.exec(note);
  if (!m) throw new Error('bad note ' + note);
  const semis = { C: -9, D: -7, E: -5, F: -4, G: -2, A: 0, B: 2 }[m[1]] + (m[2] === '#' ? 1 : m[2] === 'b' ? -1 : 0);
  return 440 * Math.pow(2, (semis + (Number(m[3]) - 4) * 12) / 12);
}

/** A sequence of notes `[[buffer, at], ...]` from `notes` spaced `step` s apart, each made by `voice(freq, i)`. */
export function seq(notes, step, voice) {
  return mix(notes.map((note, i) => [voice(hz(note), i), i * step]));
}
