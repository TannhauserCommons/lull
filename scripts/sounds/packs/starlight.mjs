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

/** A struck glass bell: a clear tone with two inharmonic overtones that die away faster. */
const ping = (f, sec = 0.9, lvl = 1) => d.bell(sec, f, [[1, lvl, sec * 0.45], [2.76, 0.25 * lvl, sec * 0.16], [5.4, 0.07 * lvl, sec * 0.06]], 0.001);

/** `hands` claps near the moment `at`, each a few ms off the others: the more `spread`, the less the room is in step. */
function clapAt(rng, buf, at, hands, spread, level = 1) {
  for (let h = 0; h < hands; h++) {
    const hand = d.shape(d.biquad(d.noise(0.03, rng), 'bandpass', 900 + 1800 * rng(), 1.6), d.perc(0.0006, 0.012));
    d.mixInto(buf, hand, at + (rng() - 0.5) * spread, level * (0.3 + 0.7 * rng()) / Math.sqrt(hands));
  }
}

/** An owl's hoot: a low, round tone that sags a little. */
const hoot = (f, sec) => d.biquad(d.shape(d.mix([[d.osc(sec, (t) => f * (1 - 0.05 * t / sec)), 0, 1], [d.osc(sec, f * 2), 0, 0.2]]), d.ahr(0.06, 0.15, sec)), 'lowpass', 900);

/** A crackle of embers: clicks at random, over a low breath of air. */
function crackle(rng, sec) {
  const buf = new Float32Array(d.n(sec));
  for (let i = 0; i < sec * 55; i++) {
    const c = d.shape(d.biquad(d.noise(0.02, rng), 'bandpass', 1200 + 3800 * rng(), 2.5), d.perc(0.0003, 0.004 + 0.012 * rng()));
    d.mixInto(buf, c, sec * Math.pow(rng(), 0.8), 0.2 + 0.8 * rng() * rng());
  }
  const air = d.shape(d.biquad(d.pink(sec, rng), 'lowpass', 700), d.arch(sec));
  return d.shape(d.mix([[buf, 0, 1], [air, 0, 0.55]]), d.arch(sec));
}

/** The eggs Starlight has and no other design does (docs/themes.md section 13): the opening night's own surprises. */
export const eggs = [
  // a masthead tapped three times quickly: the sky lets a star fall
  { name: 'shooting-star', when: [{ gesture: { on: 'brand', taps: 3, within: 1.5 } }], line: 'Make a wish', for: 5, once: 'always' },
  // a song named for the end of a show: the house calls for one more
  { name: 'encore', when: [{ words: { in: ['title'], any: ['encore', 'encores', 'bis', 'rappel', 'zugabe', 'bravo', 'applause', 'ovation', 'finale', 'curtain', 'overture', 'アンコール', '앙코르'] } }], line: 'Encore', once: 'song' },
  // half past seven, curtain time: while a song plays, the house lights go down and the footlights come up
  { name: 'curtain-up', when: [{ clock: { from: '19:30', to: '19:32' } }, { player: { playing: true } }], line: 'Curtain up', once: 'day' },
  // now and then a like is answered from the stalls
  { name: 'bouquet', when: [{ event: { on: 'liked' } }, { chance: { one_in: 20 } }], line: 'Bravo', for: 6, once: 'always' },
];

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
      // the words a title can carry: each a small sound of the thing, as a poster would print it
      moon: d.room(d.mix([[d.bell(1.8, d.hz('A4'), [[1, 1, 0.9], [2.01, 0.3, 0.5], [3, 0.12, 0.3]], 0.04), 0, 0.8], [d.bell(1.8, d.hz('E5'), [[1, 1, 0.9], [2.01, 0.3, 0.5]], 0.04), 0.28, 0.6]]), 0.9, 0.5, 0.4, 1.2),
      rain: (() => {
        const drops = [];
        for (let i = 0; i < 7; i++) drops.push([d.shape(d.osc(0.1, (t) => (500 + 600 * rng()) * (1 + 0.8 * t)), d.perc(0.001, 0.045)), 1.2 * rng(), 0.12 + 0.12 * rng()]);
        return d.room(d.mix([[rustle(rng, 1.4, 320, 3000, 9000), 0, 0.8], ...drops]), 0.6, 0.5, 0.25, 0.4);
      })(),
      star: d.room(d.mix([[ping(d.hz('E6'), 0.9), 0, 0.7], [ping(d.hz('G#6'), 0.9), 0.09, 0.6], [ping(d.hz('B6'), 1.1), 0.19, 0.5], [d.shape(d.biquad(d.noise(0.2, rng), 'highpass', 6000), d.perc(0.002, 0.1)), 0.18, 0.15]]), 0.7, 0.4, 0.3, 0.8),
      sun: d.room(d.mix([[pad(rng, 1.7, ['D3', 'A3', 'F#4', 'D5']), 0, 0.9], [ping(d.hz('A5'), 1.0, 0.8), 0.7, 0.25]]), 0.8, 0.5, 0.35, 0.8),
      snow: d.room(d.mix([[ping(d.hz('E6'), 2, 0.4), 0, 1], [ping(d.hz('B6'), 2, 0.3), 0.35, 1], [ping(d.hz('G#6'), 2, 0.25), 0.7, 1], [d.shape(d.biquad(d.pink(1.8, rng), 'lowpass', 2200), d.arch(1.8)), 0, 0.12]]), 0.95, 0.6, 0.55, 1.4),
      night: d.room(d.mix([[hoot(d.hz('A3'), 0.32), 0, 1], [hoot(d.hz('F3'), 0.5), 0.42, 0.9], [pad(rng, 1.6, ['D3', 'A3']), 0, 0.3]]), 0.9, 0.55, 0.45, 1),
      sea: (() => {
        const sec = 2.4, hush = d.arch(sec);
        const swell = d.shape(d.biquad(d.pink(sec, rng), 'lowpass', (t) => 300 + 2200 * Math.pow(hush(t), 2), 0.9), (t) => Math.pow(hush(t), 1.5));
        const foam = d.shape(d.biquad(d.noise(sec, rng), 'highpass', 4000), (t) => 0.25 * Math.pow(Math.max(0, Math.sin(Math.PI * (t - 0.5) / 1.6)), 3));
        return d.mix([[swell, 0, 1], [foam, 0, 0.5]]);
      })(),
      storm: d.room(d.mix([
        [d.shape(d.biquad(d.biquad(d.noise(0.3, rng), 'highpass', 1500), 'lowpass', 5000), d.perc(0.002, 0.08)), 0, 0.3],
        [d.shape(d.biquad(d.brown(2.6, rng), 'lowpass', 320), (t) => Math.min(1, t / 0.15) * Math.exp(-t * 1.3)), 0.02, 5],
      ]), 0.8, 0.5, 0.3, 0.5),
      fire: crackle(rng, 1.8),
      dream: d.room(d.seq(['D5', 'E5', 'F#5', 'A5', 'B5', 'D6', 'E6', 'F#6'], 0.075, (f) => d.shape(d.pluck(1.4, f, rng, 0.25, 0.997), d.perc(0.002, 1))), 0.95, 0.45, 0.5, 1.4),
      // ten plays in a row: the ticket punched ten times, quicker as it goes
      'on-repeat': d.room(d.mix(Array.from({ length: 10 }, (_, i) => [punch(rng, 0.9 + 0.04 * i), 0.1 * i + 0.012 * i * i * 0.1 + 0.06 * i, 0.55 + 0.045 * i])), 0.6, 0.4, 0.2, 0.5),
      // Starlight's own
      'shooting-star': d.room(d.mix([
        [d.shape(d.osc(1.3, (t) => 700 + 2800 * Math.exp(-t * 2.6)), (t) => 0.12 * (t < 0.03 ? t / 0.03 : Math.exp(-(t - 0.03) * 3.2))), 0, 1],
        [d.shape(d.biquad(d.noise(1.2, rng), 'bandpass', (t) => 6000 - 3600 * t, 1.2), d.arch(1.2)), 0, 0.12],
        [ping(d.hz('B6'), 1.4, 0.5), 0.45, 0.4], [ping(d.hz('F#6'), 1.6, 0.4), 0.62, 0.35],
      ]), 0.8, 0.45, 0.35, 0.9),
      // the whole house clapping in time, a little ragged, getting quicker
      encore: (() => {
        const sec = 4.2, buf = new Float32Array(d.n(sec + 0.5));
        let at = 0.1, gap = 0.44, k = 0;
        while (at < sec - 0.3) { clapAt(rng, buf, at, 16, 0.004 + 0.006 * k, 1); at += gap; gap = Math.max(0.27, gap * 0.955); k++; }
        return d.room(d.shape(buf, (t) => Math.min(1, t / 0.5) * Math.min(1, Math.max(0, (sec - t) / 1))), 0.9, 0.45, 0.4, 1);
      })(),
      // the room hushes, the curtain swishes up, and strings open on D major
      'curtain-up': d.room(d.mix([
        [d.shape(murmur(rng, 1.6, 6), (t) => Math.max(0, 1 - t / 1.4)), 0, 0.7],
        [d.shape(d.biquad(d.pink(1.1, rng), 'bandpass', (t) => 250 + 2600 * Math.pow(t / 1.1, 2), 1.2), d.arch(1.1)), 0.9, 0.55],
        [pad(rng, 2.2, ['D3', 'A3', 'D4', 'F#4', 'A4']), 1.7, 0.9],
        [ping(d.hz('D6'), 1.5, 0.35), 2.0, 0.3],
      ]), 0.85, 0.45, 0.3, 1),
      // a rose thrown from the stalls: the whoosh of its fall, the soft thud, and cellophane crinkling
      bouquet: (() => {
        const crinkle = new Float32Array(d.n(0.7));
        for (let i = 0; i < 26; i++) d.mixInto(crinkle, d.shape(d.biquad(d.noise(0.012, rng), 'bandpass', 3000 + 4000 * rng(), 1.8), d.perc(0.0005, 0.004 + 0.006 * rng())), 0.6 * Math.pow(rng(), 1.3), 0.25 + 0.75 * rng());
        return d.room(d.mix([
          [d.shape(d.biquad(d.noise(0.5, rng), 'lowpass', (t) => 2400 - 1900 * t / 0.5), d.arch(0.5)), 0, 0.25],
          [d.shape(d.osc(0.12, (t) => 150 + 70 * Math.exp(-t * 50)), d.perc(0.002, 0.07)), 0.5, 0.9],
          [crinkle, 0.52, 0.55],
          [ping(d.hz('E6'), 1, 0.2), 0.85, 0.3],
        ]), 0.6, 0.45, 0.25, 0.6);
      })(),
    }
  };
}
