// Skydeck, the star wheel: paper cards that click, a wheel's soft ratchet, a radio dial tuning through static.
import * as d from '../dsp.mjs';

export const meta = { id: 'skydeck', name: 'Star wheel', description: 'Paper cards, a soft ratchet, and a radio dial in the static.' };

/** A paper card's click: a dry snap with a small papery body. */
function card(rng, bright = 2600, body = 420) {
  const snap = d.shape(d.biquad(d.biquad(d.noise(0.04, rng), 'bandpass', bright, 1.4), 'highpass', 900), d.perc(0.0008, 0.018));
  const tock = d.shape(d.osc(0.04, body), d.perc(0.001, 0.02));
  return d.mix([[snap, 0, 1], [tock, 0, 0.35]]);
}

/** One tooth of the wheel's ratchet. */
const tooth = (rng, f = 3400) => d.mix([
  [d.shape(d.biquad(d.noise(0.02, rng), 'bandpass', f, 3), d.perc(0.0005, 0.008)), 0, 1],
  [d.shape(d.osc(0.02, f * 0.53), d.perc(0.0005, 0.01)), 0, 0.25]
]);

/** The wheel turning: `count` teeth, slowing (or speeding, with a negative `ease`) as it goes. */
function ratchet(rng, count, gap = 0.035, ease = 0.12, fall = 0.85) {
  const parts = [];
  let at = 0;
  for (let i = 0; i < count; i++) { parts.push([tooth(rng, 3200 + 300 * rng()), at, Math.pow(fall, i)]); at += gap * (1 + ease * i); }
  return d.mix(parts);
}

/** A star: a small high bell. */
const star = (f, sec = 1.2) => d.bell(sec, f, [[1, 1, 0.5], [2.76, 0.35, 0.18], [5.4, 0.12, 0.06]], 0.001);

/** The dial between stations: static whose colour follows the dial, and whistles where it passes a carrier. */
function dial(rng, sec, stations) {
  const centre = (t) => 1300 + 700 * Math.sin(2 * Math.PI * t / sec) + 250 * Math.sin(2 * Math.PI * 3 * t / sec);
  const stat = d.biquad(d.pink(sec, rng), 'bandpass', centre, 1.1);
  d.mixInto(stat, d.biquad(d.noise(sec, rng), 'bandpass', (t) => centre(t) * 2.2, 2), 0, 0.15);
  const buf = d.shape(stat, (t) => 0.7 + 0.3 * Math.sin(2 * Math.PI * 5 * t / sec));
  for (const at of stations) {
    // a heterodyne whistle: the carrier beating against the dial, falling to nothing as the dial passes it
    const w = d.shape(d.osc(0.9, (t) => 2400 * Math.pow(1 - t / 0.9, 2) + 60), d.arch(0.9));
    d.mixInto(buf, w, at, 0.08);
  }
  return d.biquad(d.biquad(buf, 'highpass', 250), 'lowpass', 4500);
}

export function sounds(rng) {
  const longDial = () => {
    const sec = 16, base = dial(rng, sec, [1.5, 6.2]);
    // the static thins as a faint, far-off tune comes in on an AM station: band-limited, slightly wavering
    d.shape(base, (t) => 1 - 0.65 * Math.min(1, Math.max(0, (t - 3) / 6)) * (t < 14 ? 1 : (16 - t) / 2));
    const tune = d.seq(['D5', 'F#5', 'A5', 'G5', 'F#5', 'E5', 'D5', 'A4', 'B4', 'D5'], 0.9,
      (f) => d.shape(d.osc(1.1, (t) => f * (1 + 0.004 * Math.sin(t * 2 * Math.PI * 4)), d.tri), d.perc(0.03, 0.8)));
    d.biquad(d.biquad(tune, 'highpass', 400), 'lowpass', 2800);
    d.mixInto(base, d.shape(tune, (t) => Math.min(1, t / 3)), 4, 0.35);
    return { loop: d.seamless(base, 0.6) };
  };
  return {
    sounds: {
      boot: d.room(d.mix([
        [ratchet(rng, 10, 0.06, -0.06, 0.97), 0, 0.7],
        [d.seq(['A5', 'E6', 'B6', 'C#7'], 0.11, (f) => star(f, 1.8)), 0.55, 0.45]
      ]), 0.6, 0.45, 0.25, 1),
      'rare-boot': d.room(d.mix([
        [ratchet(rng, 14, 0.05, -0.05, 0.98), 0, 0.7],
        [d.seq(['A5', 'E6', 'B6', 'C#7', 'E7', 'B6'], 0.1, (f) => star(f, 1.8)), 0.6, 0.45]
      ]), 0.7, 0.45, 0.3, 1.1),
      play: d.mix([[card(rng, 2400, 380), 0], [card(rng, 3000, 460), 0.028, 0.7]]),
      pause: card(rng, 1900, 300),
      skip: d.mix([
        [d.shape(d.biquad(d.noise(0.09, rng), 'bandpass', (t) => 1200 + 40000 * t, 1.5), d.arch(0.09)), 0, 0.5],
        [card(rng, 2800, 420), 0.08, 1]
      ]),
      seek: ratchet(rng, 4, 0.03, 0.25, 0.75),
      buffering: { loop: d.seamless(dial(rng, 8.4, [2.1, 5.8]), 0.4) },
      'long-wait': longDial(),
      found: d.mix([
        [d.shape(d.osc(0.45, (t) => 900 + 700 * Math.exp(-t * 14)), d.perc(0.01, 0.32)), 0, 0.35],
        [card(rng, 3200, 520), 0.3, 0.8]
      ]),
      stalled: d.shape(d.biquad(d.pink(0.6, rng), 'bandpass', (t) => 1600 - 1800 * t, 1.2), d.perc(0.02, 0.45)),
      saved: d.mix([
        [d.shape(d.biquad(d.noise(0.12, rng), 'bandpass', 2600, 1), d.arch(0.12)), 0, 0.35],
        [d.shape(d.osc(0.08, 140), d.perc(0.002, 0.05)), 0.1, 0.5],
        [card(rng, 2200, 360), 0.11, 0.9]
      ]),
      liked: d.room(d.mix([[star(d.hz('E6')), 0], [star(d.hz('B6')), 0.07, 0.7]]), 0.5, 0.4, 0.3, 0.7),
      queued: d.mix([[d.shape(d.biquad(d.noise(0.05, rng), 'bandpass', 3500, 1.2), d.arch(0.05)), 0, 0.35], [card(rng, 2600, 400), 0.04, 0.8]]),
      error: d.shape(d.osc(0.6, (t) => 700 - 500 * t + 30 * Math.sin(t * 2 * Math.PI * 11), d.tri), (t) => Math.exp(-t * 6) * (t < 0.005 ? t / 0.005 : 1)),
      design: ratchet(rng, 8, 0.04, 0.1, 0.88),
      classic: d.seq(['A5', 'E6'], 0.13, (f) => d.shape(d.osc(0.09, f), d.ahr(0.003, 0.02, 0.09))),
    },
    eggs: {
      'night-owl': d.room(d.shape(d.biquad(d.mix([[d.osc(1.6, d.hz('F#4'), d.tri), 0, 0.5], [d.osc(1.6, d.hz('A4'), d.tri), 0, 0.3]]), 'lowpass', 1500), d.arch(1.6)), 0.8, 0.5, 0.3, 1),
      'new-year': d.room(d.mix([[ratchet(rng, 12, 0.03, 0.05, 0.95), 0, 0.6], [d.seq(['E6', 'A6', 'C#7', 'E7'], 0.09, (f) => star(f, 1.6)), 0.4, 0.5]]), 0.7, 0.4, 0.3, 1),
      birthday: d.room(d.seq(['A5', 'C#6', 'E6', 'D6', 'C#6', 'A6'], 0.15, (f) => star(f, 1.2)), 0.6, 0.4, 0.3, 0.9),
      milestone: d.room(d.mix(Array.from({ length: 14 }, (_, i) => [star(1800 + 3200 * rng(), 1), i * 0.06 + 0.02 * rng(), 0.35])), 0.7, 0.4, 0.35, 1),
    }
  };
}
