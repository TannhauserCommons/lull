// lull, the candy desktop: gel buttons that squish, a sparkly start-up chime, a soft CRT hum while it waits.
import * as d from '../dsp.mjs';

export const meta = { id: 'lull', name: 'Candy desktop', description: 'Gel buttons that squish, sparkles, and a soft CRT hum.' };

/** A gel button: a soft blob whose pitch sinks as it is pressed, with a wet squelch and a little wobble after. */
function squish(rng, from, to, len = 0.14, squelch = 0.25) {
  const body = d.shape(d.osc(len, (t) => to + (from - to) * Math.exp(-t * 38) + 6 * Math.sin(t * 2 * Math.PI * 28) * Math.exp(-t * 20)),
    (t) => (t < 0.004 ? t / 0.004 : Math.exp(-(t - 0.004) * 30)));
  const wet = d.shape(d.biquad(d.noise(0.05, rng), 'bandpass', (t) => 1800 - 14000 * t, 3), d.perc(0.002, 0.035));
  return d.biquad(d.mix([[body, 0], [wet, 0.002, squelch]]), 'lowpass', 3200);
}

/** A sparkle: a few high glints scattered over `sec`. */
function sparkle(rng, sec, count, lo = 3200, hi = 7200) {
  const parts = [];
  for (let i = 0; i < count; i++) {
    const f = lo + (hi - lo) * rng();
    parts.push([d.shape(d.osc(0.25, f), d.perc(0.001, 0.07 + 0.1 * rng())), sec * rng(), 0.12 + 0.1 * rng()]);
  }
  return d.mix(parts);
}

/** A candy chime: a soft FM bell, sweet and round. */
const chime = (f, sec = 1.4) => d.shape(d.fm(sec, f, 3.01, 2.2, (t) => Math.exp(-t * 7)), d.perc(0.002, sec * 0.6));

/** A bubble rising: a quick upward chirp. */
const bubble = (from, to, len = 0.05) => d.shape(d.osc(len, (t) => from + (to - from) * Math.pow(t / len, 0.6)), d.perc(0.002, len * 0.7));

/** A soap bubble bursting: a wet click and a quick falling blip. */
const pop = (rng, f = 900) => d.mix([
  [d.shape(d.biquad(d.noise(0.04, rng), 'bandpass', 3400, 1.4), d.perc(0.0008, 0.02)), 0, 0.8],
  [d.shape(d.osc(0.08, (t) => f * (1 - 0.5 * t / 0.08)), d.perc(0.001, 0.05)), 0, 0.5]
]);

/** The CRT: mains hum and its harmonics, a hiss under it and a few dust crackles, looping `sec`. */
function hum(rng, sec) {
  const buf = d.mix([
    [d.osc(sec, 60), 0, 0.25], [d.osc(sec, 120), 0, 0.3], [d.osc(sec, 240), 0, 0.16], [d.osc(sec, 360), 0, 0.08], [d.osc(sec, 480), 0, 0.04]
  ]);
  d.mixInto(buf, d.biquad(d.biquad(d.pink(sec, rng), 'highpass', 2000), 'lowpass', 6000), 0, 0.05);
  for (let i = 0; i < sec * 1.6; i++) {
    d.mixInto(buf, d.shape(d.biquad(d.noise(0.01, rng), 'bandpass', 2500 + 2500 * rng(), 2), d.perc(0.0005, 0.006)), sec * rng(), 0.25 * rng());
  }
  // the screen breathing: a slow swell of the hum, whole cycles in the loop
  return d.shape(buf, (t) => 0.8 + 0.2 * Math.sin(2 * Math.PI * t * 2 / sec));
}

/** lull's own surprises (names of their own, never one of eggs.mjs): the same declarations any pack makes. */
export const eggs = [
  // tap the logo four times in two seconds: the windows shake, as a friend's did when they nudged you
  { name: 'nudge', when: [{ gesture: { on: 'brand', taps: 4, within: 2 } }], line: 'Nudge!', for: 3, once: 'session' },
  // left alone with music playing: soap bubbles drift up the desk, like the old screen saver
  { name: 'bubbles', when: [{ player: { playing: true, idle: 120 } }], once: 'song' },
  // a title that talks about tomorrow: a glint crosses the glass
  { name: 'future', when: [{ words: { in: ['title'], any: ['future', 'futuro', 'futur', 'zukunft', 'toekomst', 'framtid', 'przyszłość', 'будущее', 'gelecek', '未来', '未來', '미래', 'digital', 'digitale', 'cyber', 'robot', 'robots', 'chrome', 'neon', 'electric', 'virtual', 'online', 'hologram', 'y2k', 'millennium'] } }], once: 'song' },
  // nothing playing and nobody touching anything: a little orb looks in, as the office helpers once did
  { name: 'buddy', when: [{ player: { status: 'idle', screen: 'home', idle: 90 } }], line: 'Quiet in here. Shall we pick something?', once: 'session' }
];

export function sounds(rng) {
  const bootChime = (notes) => d.room(d.mix([
    [squish(rng, 520, 160, 0.2, 0.2), 0, 0.6],
    [d.seq(notes, 0.085, (f) => chime(f, 1.6)), 0.12, 0.55],
    [sparkle(rng, 1.6, 18), 0.25, 1]
  ]), 0.7, 0.35, 0.3, 1.1);
  const longHum = () => {
    const sec = 14.4, base = hum(rng, sec);
    ['G5', 'B5', 'D6', 'F#6', 'E6', 'D6', 'B5', 'A5'].forEach((note, i) => d.mixInto(base, chime(d.hz(note), 1.2), 1 + i * 1.55, 0.06 + i * 0.008));
    return { loop: d.seamless(base, 0.5) };
  };
  return {
    sounds: {
      boot: bootChime(['Eb5', 'G5', 'Bb5', 'D6', 'F6']),
      'rare-boot': bootChime(['F6', 'D6', 'Bb5', 'G5', 'Eb5', 'G5', 'Bb5', 'Eb6']),
      play: squish(rng, 680, 190),
      pause: squish(rng, 880, 360, 0.1, 0.18),
      skip: d.mix([[squish(rng, 760, 260, 0.09, 0.15), 0], [squish(rng, 900, 330, 0.09, 0.15), 0.07, 0.8]]),
      seek: bubble(1200, 2100, 0.03),
      buffering: { loop: d.seamless(hum(rng, 6.4), 0.4) },
      'long-wait': longHum(),
      found: d.mix([[bubble(700, 1300), 0, 0.8], [bubble(1000, 1900), 0.06, 0.8], [sparkle(rng, 0.25, 4), 0.08, 1]]),
      stalled: d.shape(d.osc(0.45, (t) => 150 + 380 * Math.exp(-t * 6) + 12 * Math.sin(t * 2 * Math.PI * 9)), d.perc(0.006, 0.35)),
      saved: d.room(d.mix([[d.seq(['C6', 'E6', 'A6'], 0.075, (f) => chime(f, 1.1)), 0], [sparkle(rng, 0.5, 6), 0.15, 1]]), 0.5, 0.4, 0.25, 0.7),
      liked: d.room(d.mix([
        [d.shape(d.osc(0.3, (t) => 520 + 420 * (1 - Math.exp(-t * 30)) + 40 * Math.sin(t * 2 * Math.PI * 14) * Math.exp(-t * 8)), d.perc(0.003, 0.22)), 0, 0.7],
        [sparkle(rng, 0.4, 7, 4000, 8000), 0.05, 1]
      ]), 0.5, 0.4, 0.2, 0.6),
      queued: bubble(600, 1100, 0.045),
      error: d.mix([
        [d.shape(d.osc(0.2, (t) => 140 + 90 * Math.exp(-t * 25) + 8 * Math.sin(t * 2 * Math.PI * 22)), d.perc(0.003, 0.14)), 0],
        [d.shape(d.osc(0.26, (t) => 110 + 70 * Math.exp(-t * 25) + 8 * Math.sin(t * 2 * Math.PI * 22)), d.perc(0.003, 0.2)), 0.16]
      ]),
      design: d.room(d.mix([[sparkle(rng, 0.6, 14, 2500, 8000), 0], [chime(d.hz('Bb5'), 1), 0.4, 0.4]]), 0.6, 0.4, 0.3, 0.8),
      classic: d.seq(['C5', 'G5'], 0.11, (f) => d.shape(d.osc(0.18, f, d.tri), d.perc(0.003, 0.12))),
    },
    eggs: {
      'night-owl': d.room(d.shape(d.osc(1.4, (t) => 620 - 280 * Math.min(1, t / 1.2) + 10 * Math.sin(t * 2 * Math.PI * 5)), d.arch(1.4)), 0.7, 0.5, 0.35, 1),
      'new-year': d.room(d.mix([[sparkle(rng, 1.5, 40, 2500, 9000), 0], [d.seq(['Eb5', 'Bb5', 'Eb6', 'G6'], 0.1, (f) => chime(f, 1.5)), 0.2, 0.5]]), 0.8, 0.35, 0.35, 1.2),
      birthday: d.room(d.mix([[d.seq(['Bb5', 'Eb6', 'G6', 'F6', 'Eb6'], 0.16, (f) => chime(f, 1.1)), 0, 0.6], [sparkle(rng, 1, 10), 0.2, 1]]), 0.6, 0.4, 0.3, 0.9),
      milestone: d.room(d.mix([
        [squish(rng, 600, 200, 0.16), 0, 0.6],
        [d.seq(['Eb5', 'G5', 'Bb5', 'Eb6', 'G6'], 0.07, (f) => chime(f, 1.4)), 0.1, 0.55],
        [sparkle(rng, 1.2, 30, 3000, 9000), 0.2, 1]
      ]), 0.7, 0.35, 0.3, 1.1),
      nudge: d.room(d.mix([
        [squish(rng, 440, 170, 0.17, 0.2), 0, 0.7], [squish(rng, 540, 200, 0.17, 0.2), 0.15, 0.7],
        [d.shape(d.osc(0.7, (t) => 1500 + 110 * Math.sin(t * 2 * Math.PI * 26)), d.perc(0.002, 0.5)), 0.02, 0.1],
        [sparkle(rng, 0.7, 9, 3500, 7500), 0.05, 0.8]
      ]), 0.4, 0.4, 0.25, 0.5),
      // bubbles: a soft rising shimmer, and seven pops scattered over three seconds
      bubbles: (() => {
        const sec = 3.4, buf = d.mix([[d.shape(d.biquad(d.pink(sec, rng), 'bandpass', (t) => 900 + 1600 * t / sec, 1.2), d.arch(sec)), 0, 0.35]]);
        [0.3, 0.75, 1.15, 1.6, 2.05, 2.45, 2.95].forEach((at, i) => d.mixInto(buf, pop(rng, 700 + 140 * ((i * 5) % 4)), at, 0.5 + 0.3 * rng()));
        return d.room(buf, 0.6, 0.4, 0.3, 0.8);
      })(),
      // future: a bright sweep and a soft major ninth, the way a new system said hello
      future: d.room(d.mix([
        [d.shape(d.biquad(d.pink(1.3, rng), 'bandpass', (t) => 500 + 5500 * Math.pow(t / 1.3, 1.6), 2.4), d.arch(1.3)), 0, 0.9],
        [d.seq(['Eb5', 'G5', 'Bb5', 'D6', 'F6'], 0.11, (f) => chime(f, 1.8)), 0.5, 0.5],
        [sparkle(rng, 1.8, 16, 3000, 9000), 0.7, 0.8]
      ]), 0.8, 0.35, 0.35, 1.2),
      // buddy: two soft questioning blips, the second higher
      buddy: d.room(d.mix([[bubble(500, 760, 0.09), 0, 0.9], [bubble(700, 1150, 0.12), 0.16, 0.9], [sparkle(rng, 0.4, 3, 4000, 7000), 0.2, 0.7]]), 0.4, 0.4, 0.2, 0.5),
      // a disc spinning up: a rising whirr with a sparkle on top
      'on-repeat': d.room(d.mix([
        [d.shape(d.biquad(d.pink(1.2, rng), 'bandpass', (t) => 300 + 2400 * Math.pow(t / 1.2, 1.4), 3), d.arch(1.2)), 0, 1],
        [d.seq(['G5', 'D6'], 0.14, (f) => chime(f, 1.2)), 0.8, 0.5],
        [sparkle(rng, 0.8, 8, 3500, 8000), 0.9, 0.8]
      ]), 0.5, 0.4, 0.25, 0.7)
    }
  };
}
