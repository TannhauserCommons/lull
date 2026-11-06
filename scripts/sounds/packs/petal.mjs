// Petal, the pixel zine: 8-bit blips, a tape deck's clunk, a VHS tape loading and tracking while it waits.
import * as d from '../dsp.mjs';

export const meta = { id: 'petal', name: 'Pixel zine', description: '8-bit blips, a tape deck clunk, and a VHS tape finding its tracking.' };

/** An 8-bit note: a soft-edged square at `duty`, short and clean. */
const blip = (f, len = 0.07, duty = 0.5) => d.shape(d.osc(len, f, d.square(duty, 0.03)), d.ahr(0.002, len * 0.4, len));

/** A run of 8-bit notes. */
const run = (notes, step, len, duty) => d.seq(notes, step, (f) => blip(f, len, duty));

/** A tape deck's key going down: the latch's click, the mechanism's thump, and the motor taking up. */
function clunk(rng, motor = 1) {
  const click = d.shape(d.biquad(d.noise(0.02, rng), 'bandpass', 2800, 2), d.perc(0.0005, 0.006));
  const thump = d.shape(d.osc(0.08, (t) => 120 + 80 * Math.exp(-t * 50)), d.perc(0.001, 0.05));
  const rattle = d.shape(d.biquad(d.noise(0.05, rng), 'bandpass', 900, 3), d.perc(0.001, 0.02));
  const whir = d.shape(d.biquad(d.mix([[d.osc(0.25, (t) => (motor > 0 ? 80 + 140 * t / 0.25 : 220 - 140 * t / 0.25), d.square(0.5, 0.2)), 0, 0.3], [d.noise(0.25, rng), 0, 0.15]]), 'lowpass', 900), d.arch(0.25));
  return d.mix([[click, 0, 0.8], [thump, 0.004, 0.9], [rattle, 0.006, 0.4], [whir, 0.02, 0.35]]);
}

/** A VCR: the drum's hum, the motor's wow, and bursts of tracking noise rolling up the picture. */
function vcr(rng, sec) {
  const drum = d.biquad(d.mix([[d.osc(sec, (t) => 59.94 * (1 + 0.01 * Math.sin(2 * Math.PI * t * 0.7))), 0, 0.5], [d.osc(sec, 119.88), 0, 0.4], [d.osc(sec, 239.76), 0, 0.2], [d.osc(sec, 359.64), 0, 0.1]]), 'lowpass', 800);
  const hiss = d.shape(d.biquad(d.noise(sec, rng), 'bandpass', 5200, 0.8), (t) => 0.1 + 0.04 * Math.sin(2 * Math.PI * t * 1.3));
  const buf = d.mix([[drum, 0, 0.45], [hiss, 0, 0.8]]);
  for (let at = 0.6; at < sec - 0.8; at += 1.6 + 0.6 * rng()) {
    const burst = d.shape(d.biquad(d.noise(0.5, rng), 'bandpass', (t) => 800 + 5000 * t / 0.5, 1.4), d.arch(0.5));
    d.mixInto(buf, burst, at, 0.3);
    // the heads catching the control track: a tiny clatter
    d.mixInto(buf, d.shape(d.biquad(d.noise(0.02, rng), 'bandpass', 1800, 3), d.perc(0.0005, 0.01)), at + 0.48, 0.25);
  }
  return buf;
}

/** Petal's own surprises (they come after lull's own, and only this design has them): their looks are in
    web/packs/petal/pack.css and slots/, their sounds below under the same names. */
export const eggs = [
  // three quick taps on the song's title: the bunny hops across the window
  { name: 'bunny-hop', when: [{ gesture: { on: 'title', taps: 3, within: 2 } }], line: 'hop hop hop', for: 6, once: 'always' },
  // a like, now and then: a sheet of stickers rains down
  { name: 'sticker-rain', when: [{ event: { on: 'liked' } }, { chance: { one_in: 8 } }], line: 'Sticker sheet, just for you', for: 6, once: 'session' },
  // the tape ran out: the cassette flips to its other side
  { name: 'side-b', when: [{ player: { status: 'ended' } }], line: 'Flip to side B', once: 'song' },
  // left alone with music anywhere but Now Playing: doodles appear in the paper's margin
  { name: 'margin-doodle', when: [{ player: { playing: true, idle: 90, screen: ['home', 'search', 'library', 'liked', 'history', 'following', 'album', 'artist', 'tag', 'playlists', 'playlist', 'stats'] } }], once: 'song' },
];

export function sounds(rng) {
  const longVcr = () => {
    const sec = 16, base = vcr(rng, sec);
    // the tracking settles, and an 8-bit tune plays out of the TV's small speaker
    d.shape(base, (t) => 1 - 0.55 * Math.min(1, Math.max(0, (t - 3) / 3)) * Math.min(1, Math.max(0, (15.4 - t) / 1)));
    const tune = run(['E5', 'G5', 'B5', 'A5', 'G5', 'E5', 'D5', 'E5', 'G5', 'D6', 'B5', 'A5'], 0.48, 0.36, 0.25);
    d.biquad(d.biquad(tune, 'highpass', 500), 'lowpass', 3000);
    d.mixInto(base, d.shape(tune, (t) => Math.min(1, t / 2)), 5, 0.3);
    return { loop: d.seamless(base, 0.6) };
  };
  return {
    sounds: {
      boot: d.mix([[run(['D5', 'A5', 'F#5', 'D6', 'E6', 'A6'], 0.075, 0.1, 0.25), 0, 0.7], [clunk(rng), 0.5, 0.6]]),
      'rare-boot': d.mix([[run(['D5', 'A5', 'F#5', 'D6', 'E6', 'A6', 'F#6', 'D7'], 0.065, 0.09, 0.125), 0, 0.65], [clunk(rng), 0.58, 0.6]]),
      play: clunk(rng, 1),
      pause: clunk(rng, -1),
      skip: run(['E6', 'A6'], 0.045, 0.04, 0.25),
      seek: blip(1500, 0.018, 0.5),
      buffering: { loop: d.seamless(vcr(rng, 8.4), 0.4) },
      'long-wait': longVcr(),
      found: run(['C6', 'G6', 'C7'], 0.05, 0.05, 0.5),
      stalled: d.shape(d.osc(0.5, (t) => 420 * Math.pow(0.4, t / 0.5) * (1 + 0.03 * Math.sin(2 * Math.PI * t * 7)), d.square(0.5, 0.2)), d.ahr(0.005, 0.2, 0.5)),
      saved: d.mix([[run(['G5', 'D6'], 0.06, 0.06, 0.5), 0, 0.7], [clunk(rng), 0.14, 0.5]]),
      liked: run(['A5', 'C#6', 'E6', 'A6'], 0.04, 0.06, 0.25),
      queued: blip(1100, 0.04, 0.25),
      error: d.shape(d.osc(0.3, (t) => 300 - 160 * t / 0.3, d.square(0.5, 0.05)), d.ahr(0.003, 0.08, 0.3)),
      design: d.shape(d.osc(0.3, (t) => 300 * Math.pow(8, t / 0.3), d.square(0.25, 0.05)), d.ahr(0.003, 0.08, 0.3)),
      classic: d.mix([
        [clunk(rng, -1), 0, 0.7],
        [d.shape(d.osc(0.2, (t) => 900 + 300 * Math.sin(2 * Math.PI * t * 30) * Math.exp(-t * 12)), d.perc(0.002, 0.12)), 0.08, 0.15]
      ]),
    },
    eggs: {
      'night-owl': run(['E5', 'C5', 'A4'], 0.38, 0.32, 0.5),
      'new-year': d.mix([[run(['A5', 'A5', 'A5'], 0.25, 0.06, 0.5), 0], [run(['A5', 'C#6', 'E6', 'A6', 'E6', 'A6'], 0.08, 0.12, 0.25), 0.8, 1]]),
      birthday: run(['D5', 'F#5', 'A5', 'G5', 'F#5', 'D6'], 0.14, 0.12, 0.25),
      milestone: run(['C5', 'E5', 'G5', 'C6', 'G5', 'C6', 'E6', 'G6'], 0.07, 0.1, 0.25),
      // ten times in a row: the tape rewinds, and the key clunks down
      'on-repeat': d.mix([
        [d.shape(d.mix([[d.osc(1.1, (t) => 260 + 1500 * Math.pow(t / 1.1, 1.5), d.square(0.5, 0.2)), 0, 0.3], [d.biquad(d.noise(1.1, rng), 'bandpass', 2400, 0.8), 0, 0.25]]), d.arch(1.1)), 0, 0.8],
        [clunk(rng, -1), 1.12, 0.7],
      ]),
      // eleven hops, each a rising blip, the tune climbing as the bunny crosses
      'bunny-hop': d.mix([
        ...['C5', 'D5', 'E5', 'G5', 'A5', 'C6', 'D6', 'E6', 'G6', 'A6', 'C7'].map((nt, i) => [
          d.shape(d.osc(0.16, (t) => d.hz(nt) * (0.82 + 0.36 * t / 0.16), d.square(0.25, 0.03)), d.ahr(0.002, 0.09, 0.16)), i * 0.45, 0.7]),
        ...Array.from({ length: 11 }, (_, i) => [d.shape(d.biquad(d.noise(0.02, rng), 'bandpass', 1800, 2), d.perc(0.0005, 0.01)), i * 0.45 + 0.24, 0.3]),
      ]),
      // a sheet of stickers: a sparkle running down, and a last pop
      'sticker-rain': d.mix([
        [d.echo(run(['G7', 'E7', 'D7', 'C7', 'A6', 'G6', 'E6', 'D6', 'C6'], 0.09, 0.1, 0.125), 0.18, 0.4, 2), 0, 0.8],
        [blip(1500, 0.03, 0.5), 1.05, 0.5],
      ]),
      // the cassette flips: clunk, whir, clunk, and a two-note "B"
      'side-b': d.mix([
        [clunk(rng, 1), 0, 0.9],
        [d.shape(d.mix([[d.osc(0.5, (t) => 520 - 360 * Math.sin(Math.PI * t / 0.5), d.square(0.5, 0.2)), 0, 0.25], [d.biquad(d.noise(0.5, rng), 'bandpass', 1500, 1), 0, 0.2]]), d.arch(0.5)), 0.2, 0.7],
        [clunk(rng, -1), 0.75, 0.9],
        [run(['A5', 'E6'], 0.1, 0.12, 0.25), 1.45, 0.6],
      ]),
      // pencil on paper: five quick scribbles, one for each doodle, each ending in a tick
      'margin-doodle': d.mix(Array.from({ length: 5 }, (_, i) => [
        d.mix([[d.shape(d.biquad(d.noise(0.4, rng), 'bandpass', 3800 + 300 * i, 1.2), (t) => d.arch(0.4)(t) * (0.55 + 0.45 * Math.sin(2 * Math.PI * t * 17))), 0, 0.8], [blip(1800 + 120 * i, 0.03, 0.5), 0.42, 0.3]]), i * 1.1, 0.8])),
    }
  };
}
