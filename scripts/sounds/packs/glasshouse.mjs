// Glasshouse, after-midnight aero: glass chimes, a start-up swell from a glossy 2000s desktop, soft wind.
import * as d from '../dsp.mjs';

export const meta = { id: 'glasshouse', name: 'After midnight', description: 'Glass chimes, a glossy start-up swell, and soft wind.' };

/** Glass: a struck pane or bowl, inharmonic and clear, with a long gentle ring. */
const glass = (f, sec = 1.6, ring = 1) => d.bell(sec, f, [[1, 1, 0.9 * ring], [2.32, 0.5, 0.45 * ring], [4.25, 0.3, 0.2 * ring], [6.63, 0.12, 0.1 * ring]], 0.001);

/** A glossy pad: detuned voices whose filter opens, swelling in and out over `sec`. */
const swell = (rng, sec, notes, open = 2600) => d.mix(notes.map((note, i) => [
  d.shape(d.biquad(d.mix([
    [d.osc(sec, d.hz(note) * 0.997, d.tri, rng()), 0, 1], [d.osc(sec, d.hz(note) * 1.003, d.tri, rng()), 0, 1], [d.osc(sec, d.hz(note) * 2, d.sine, rng()), 0, 0.25]
  ]), 'lowpass', (t) => 400 + open * Math.sin(Math.PI * Math.min(1, t / sec))), d.ahr(sec * 0.45, sec * 0.45, sec)),
  0, 1 / notes.length
]));

/** Wind: soft noise whose colour and strength drift in slow gusts. */
function wind(rng, sec) {
  const g = (t) => 0.5 + 0.3 * Math.sin(2 * Math.PI * t * 2 / sec) + 0.2 * Math.sin(2 * Math.PI * t * 5 / sec + 1);
  const low = d.biquad(d.brown(sec, rng), 'lowpass', (t) => 300 + 500 * g(t), 0.8);
  const high = d.biquad(d.pink(sec, rng), 'bandpass', (t) => 700 + 900 * g(t), 2.5);
  return d.shape(d.mix([[low, 0, 0.7], [high, 0, 0.3]]), g);
}

export function sounds(rng) {
  const glints = (sec, count, lo, hi) => d.mix(Array.from({ length: count }, () => [glass(lo + (hi - lo) * rng(), 0.9, 0.5), sec * rng(), 0.12 + 0.1 * rng()]));
  const longWind = () => {
    const sec = 16, base = wind(rng, sec);
    // the wind finds the chimes on the balcony: a slow melody of glass comes and goes in it
    ['F#5', 'A5', 'C#6', 'B5', 'A5', 'E5', 'F#5', 'C#6', 'E6'].forEach((note, i) => d.mixInto(base, glass(d.hz(note), 2.2), 2 + i * 1.35, 0.12 + 0.02 * Math.sin(i)));
    return { loop: d.seamless(base, 0.6) };
  };
  return {
    sounds: {
      boot: d.room(d.mix([[swell(rng, 3, ['D3', 'A3', 'E4', 'F#4', 'C#5']), 0, 0.9], [glints(1.6, 10, 2400, 5200), 0.9, 1], [glass(d.hz('A6'), 2), 2.2, 0.35]]), 0.85, 0.4, 0.4, 1),
      'rare-boot': d.room(d.mix([[swell(rng, 3.2, ['B2', 'F#3', 'D4', 'A4', 'C#5', 'E5']), 0, 0.9], [glints(2, 16, 2000, 6000), 0.8, 1], [glass(d.hz('F#6'), 2), 2.4, 0.35]]), 0.85, 0.4, 0.4, 1),
      play: d.room(glass(d.hz('E6'), 0.8, 0.5), 0.5, 0.3, 0.25, 0.6),
      pause: d.room(glass(d.hz('B5'), 0.6, 0.4), 0.5, 0.3, 0.25, 0.5),
      skip: d.room(d.mix([[glass(d.hz('C#6'), 0.5, 0.3), 0], [glass(d.hz('F#6'), 0.5, 0.3), 0.06, 0.8]]), 0.5, 0.3, 0.2, 0.5),
      seek: glass(d.hz('A6'), 0.25, 0.2),
      buffering: { loop: d.seamless(wind(rng, 10.4), 0.5) },
      'long-wait': longWind(),
      found: d.room(d.seq(['A5', 'C#6', 'E6'], 0.07, (f) => glass(f, 1, 0.5)), 0.6, 0.3, 0.3, 0.8),
      stalled: d.shape(wind(rng, 0.9), (t) => (t < 0.15 ? t / 0.15 : Math.exp(-(t - 0.15) * 5))),
      saved: d.room(glass(d.hz('D5'), 1.8, 1), 0.7, 0.3, 0.3, 0.9),
      liked: d.room(d.mix([[glass(d.hz('F#6'), 1.1, 0.6), 0], [glass(d.hz('C#7'), 1.1, 0.6), 0.08, 0.7]]), 0.6, 0.3, 0.3, 0.8),
      queued: glass(d.hz('E7'), 0.35, 0.25),
      error: d.mix([[glass(d.hz('D4') * 1.02, 0.6, 0.25), 0], [glass(d.hz('C#4'), 0.7, 0.25), 0.012, 0.8]]),
      design: d.room(d.mix([[d.shape(wind(rng, 0.7), d.arch(0.7)), 0, 0.8], [glass(d.hz('A5'), 0.7, 0.4), 0.4, 0.4]]), 0.6, 0.4, 0.25, 0.4),
      classic: d.room(swell(rng, 0.9, ['A3', 'E4', 'C#5'], 1800), 0.6, 0.4, 0.3, 0.6),
    },
    eggs: {
      'night-owl': d.room(d.seq(['A4', 'E4', 'A4'], 0.9, (f) => glass(f, 2, 1)), 0.9, 0.35, 0.4, 1.2),
      'new-year': d.room(d.mix([[glints(2.2, 30, 2000, 7000), 0], [d.seq(['A5', 'C#6', 'E6', 'A6'], 0.12, (f) => glass(f, 1.8)), 0.3, 0.5]]), 0.85, 0.35, 0.4, 1.2),
      birthday: d.room(d.seq(['A5', 'C#6', 'E6', 'D6', 'C#6', 'A6'], 0.17, (f) => glass(f, 1.2, 0.6)), 0.7, 0.3, 0.35, 0.9),
      milestone: d.room(d.mix([[swell(rng, 1.8, ['A3', 'E4', 'C#5']), 0, 0.6], [glints(1.5, 22, 2500, 7000), 0.2, 1]]), 0.85, 0.35, 0.4, 1),
    }
  };
}
