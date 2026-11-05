// Starlight, the poster: a ticket punch, paper rustling, a theatre murmuring far off before the show.
import * as d from '../dsp.mjs';

export const meta = { id: 'starlight', name: 'Opening night', description: 'A ticket punch, paper rustle, and a theatre murmuring before the show.' };

/** A ticket punch: a sharp snap, the die's ring, and the thud of the hand. */
function punch(rng, ring = 1) {
  const snap = d.shape(d.biquad(d.noise(0.02, rng), 'highpass', 2500), d.perc(0.0004, 0.006));
  const die = d.bell(0.12, 2900 * ring, [[1, 1, 0.04], [1.62, 0.6, 0.03], [2.1, 0.4, 0.02]], 0.0005);
  const thud = d.shape(d.osc(0.06, (t) => 130 + 90 * Math.exp(-t * 60)), d.perc(0.001, 0.035));
  return d.mix([[snap, 0, 0.8], [die, 0.001, 0.25], [thud, 0, 0.8]]);
}

/** Paper rustling: grains of bright noise, `sec` long. */
function rustle(rng, sec, density = 90, lo = 1800, hi = 6000) {
  const buf = new Float32Array(d.n(sec));
  for (let i = 0; i < sec * density; i++) {
    const g = d.shape(d.biquad(d.noise(0.012, rng), 'bandpass', lo + (hi - lo) * rng(), 1.5), d.perc(0.001, 0.006 + 0.008 * rng()));
    d.mixInto(buf, g, sec * rng(), 0.2 + 0.8 * rng());
  }
  return d.shape(buf, d.arch(sec));
}

/** An audience far off: many voices as moving formants of noise, at a talking pace, in a big room. */
function murmur(rng, sec, voices = 9) {
  const buf = new Float32Array(d.n(sec));
  for (let v = 0; v < voices; v++) {
    const f1 = 350 + 300 * rng(), f2 = 1100 + 900 * rng(), rate = 2.5 + 2.5 * rng(), ph = rng(), drift = 0.3 + 0.4 * rng();
    const a = d.biquad(d.pink(sec, rng), 'bandpass', (t) => f1 * (1 + 0.25 * Math.sin(2 * Math.PI * (t * drift + ph))), 4);
    const b = d.biquad(d.pink(sec, rng), 'bandpass', (t) => f2 * (1 + 0.2 * Math.sin(2 * Math.PI * (t * drift * 1.3 + ph))), 5);
    const voice = d.mix([[a, 0, 1], [b, 0, 0.5]]);
    // syllables: a ragged rise and fall at the voice's own pace, with pauses
    d.shape(voice, (t) => { const s = Math.sin(2 * Math.PI * (t * rate + ph)); return Math.max(0, s) * (0.5 + 0.5 * Math.sin(2 * Math.PI * (t * 0.23 + v / voices))); });
    d.mixInto(buf, voice, 0, 1 / voices);
  }
  return d.biquad(d.room(buf, 0.95, 0.65, 0.55, 0), 'lowpass', 1800).subarray(0, d.n(sec));
}

/** A warm pad, for the house lights going down. */
const pad = (rng, sec, notes) => d.mix(notes.map((note, i) => [
  d.shape(d.biquad(d.mix([[d.osc(sec, d.hz(note), d.tri, rng()), 0, 1], [d.osc(sec, d.hz(note) * 1.004, d.tri, rng()), 0, 0.7]]), 'lowpass', (t) => 600 + 1400 * t / sec), d.ahr(sec * 0.4, sec * 0.35, sec)),
  0, 1 / notes.length
]));

export function sounds(rng) {
  const longMurmur = () => {
    const sec = 16, base = murmur(rng, sec);
    // the room hushes, and an oboe gives the orchestra its A while the strings tune quietly under it
    d.shape(base, (t) => 1 - 0.6 * Math.min(1, Math.max(0, (t - 4) / 4)) * Math.min(1, Math.max(0, (15 - t) / 1)));
    const oboe = d.biquad(d.shape(d.mix([[d.osc(6, d.hz('A4'), d.tri), 0, 1], [d.osc(6, d.hz('A5'), d.sine), 0, 0.3]]), d.ahr(1.2, 1.5, 6)), 'lowpass', 2400);
    const strings = d.biquad(d.shape(d.mix([[d.osc(5, d.hz('A3') * 1.003, d.tri), 0, 0.5], [d.osc(5, d.hz('E4') * 0.997, d.tri), 0, 0.4], [d.osc(5, d.hz('D4'), d.tri), 0, 0.3]]), d.ahr(1.5, 1.5, 5)), 'lowpass', 1600);
    d.mixInto(base, d.room(oboe, 0.9, 0.5, 0.4, 0).subarray(0, d.n(6)), 6.5, 0.25);
    d.mixInto(base, d.room(strings, 0.9, 0.5, 0.5, 0).subarray(0, d.n(5)), 8.5, 0.2);
    return { loop: d.seamless(base, 0.6) };
  };
  return {
    sounds: {
      boot: d.room(d.mix([[pad(rng, 2.6, ['D3', 'A3', 'F#4', 'E5']), 0, 0.8], [punch(rng, 1.1), 2.2, 0.5]]), 0.8, 0.5, 0.35, 1),
      'rare-boot': d.room(d.mix([[pad(rng, 3, ['B2', 'F#3', 'D4', 'A4', 'C#5']), 0, 0.8], [rustle(rng, 0.6, 140), 1.6, 0.4], [punch(rng, 1.1), 2.6, 0.5]]), 0.8, 0.5, 0.35, 1),
      play: punch(rng),
      pause: rustle(rng, 0.14, 160, 1500, 4500),
      skip: rustle(rng, 0.22, 140),
      seek: rustle(rng, 0.06, 200, 2500, 6000),
      buffering: { loop: d.seamless(murmur(rng, 10.4), 0.5) },
      'long-wait': longMurmur(),
      found: d.mix([[d.shape(d.osc(0.1, (t) => 120 + 60 * Math.exp(-t * 40)), d.perc(0.002, 0.06)), 0, 0.8], [d.bell(0.5, 1900, [[1, 1, 0.2], [2.4, 0.4, 0.08]]), 0.01, 0.15]]),
      stalled: d.shape(d.biquad(d.osc(0.7, (t) => d.hz('D3') - 15 * t, d.tri), 'lowpass', 900), d.ahr(0.05, 0.4, 0.7)),
      saved: d.mix([[punch(rng, 0.9), 0], [punch(rng, 1.05), 0.11, 0.7]]),
      liked: d.mix([
        [d.shape(d.osc(0.35, (t) => 2000 + 6000 * Math.pow(t / 0.35, 1.5)), (t) => 0.15 * (t / 0.35)), 0],
        [d.shape(d.biquad(d.noise(0.05, rng), 'highpass', 1500), d.perc(0.0005, 0.03)), 0.35, 0.9],
        [d.shape(d.osc(0.06, 180), d.perc(0.001, 0.03)), 0.35, 0.5]
      ]),
      queued: d.mix([[rustle(rng, 0.12, 180), 0], [punch(rng, 1.2), 0.1, 0.4]]),
      error: d.mix([
        [d.shape(d.osc(0.25, (t) => 120 + 60 * Math.exp(-t * 30)), d.perc(0.002, 0.18)), 0, 0.9],
        [rustle(rng, 0.25, 80, 600, 2500), 0.05, 0.5]
      ]),
      design: d.shape(d.biquad(d.pink(0.9, rng), 'lowpass', (t) => 300 + 3200 * Math.sin(Math.PI * t / 0.9), 0.9), d.arch(0.9)),
      classic: d.mix([[d.shape(d.biquad(d.noise(0.25, rng), 'bandpass', 120, 6), d.ahr(0.01, 0.1, 0.25)), 0, 0.5], [punch(rng, 1.3), 0, 0.5]]),
    },
    eggs: {
      'night-owl': d.room(d.shape(d.biquad(d.noise(0.8, rng), 'bandpass', (t) => 220 + 60 * Math.sin(t * 9), 12), d.arch(0.8)), 0.9, 0.5, 0.5, 1.2),
      'new-year': (() => {
        const claps = [];
        for (let i = 0; i < 70; i++) claps.push([d.shape(d.biquad(d.noise(0.03, rng), 'bandpass', 900 + 1600 * rng(), 1.8), d.perc(0.0006, 0.012)), 2.6 * Math.pow(rng(), 0.7), 0.3 + 0.7 * rng()]);
        return d.room(d.shape(d.mix(claps), d.ahr(0.4, 0.8, 2.7)), 0.85, 0.45, 0.4, 1);
      })(),
      birthday: d.room(d.seq(['D5', 'F#5', 'A5', 'G5', 'F#5', 'D6'], 0.16, (f) => d.shape(d.osc(0.6, f, d.tri), d.perc(0.004, 0.4))), 0.7, 0.4, 0.3, 0.8),
      milestone: (() => {
        const claps = [];
        for (let i = 0; i < 45; i++) claps.push([d.shape(d.biquad(d.noise(0.03, rng), 'bandpass', 1000 + 1500 * rng(), 1.8), d.perc(0.0006, 0.012)), 1.8 * Math.pow(rng(), 0.8), 0.3 + 0.7 * rng()]);
        return d.room(d.mix([[d.shape(d.mix(claps), d.ahr(0.3, 0.6, 1.9)), 0, 1], [punch(rng), 0, 0.5]]), 0.85, 0.45, 0.4, 1);
      })(),
    }
  };
}
