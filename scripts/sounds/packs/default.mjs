// The neutral pack: soft wooden taps and a gentle marimba, for any look.
import * as d from '../dsp.mjs';

export const meta = { id: 'default', name: 'Soft', description: 'Soft taps and a gentle marimba, for any look.' };

const marimba = (f, sec = 0.9) => d.bell(sec, f, [[1, 1, 0.35], [3.93, 0.22, 0.07], [9.2, 0.06, 0.025]], 0.002);
const tap = (f, len = 0.12) => d.shape(d.osc(len, (t) => f * (1 + 0.25 * Math.exp(-t * 80))), d.perc(0.002, len * 0.6));

export function sounds(rng) {
  const pad = (sec, notes, breathe) => d.mix(notes.map((note, i) => [
    d.shape(d.osc(sec, d.hz(note), d.sine, i * 0.13), (t) => 0.55 + 0.45 * Math.sin(2 * Math.PI * (t / breathe + i * 0.25))), 0, 1 / notes.length
  ]));
  const buffering = () => {
    const sec = 8.4, p = pad(sec, ['A3', 'E4', 'B4'], 4);
    d.mixInto(p, d.biquad(d.pink(sec, rng), 'lowpass', 500), 0, 0.12);
    return { loop: d.seamless(p, 0.4) };
  };
  const longMelody = () => {
    const sec = 14.4, base = pad(sec, ['A3', 'E4'], 6);
    ['E5', 'G5', 'A5', 'C6', 'A5', 'G5', 'E5', 'D5'].forEach((note, i) => d.mixInto(base, marimba(d.hz(note), 1.6), 1.2 + i * 1.5, 0.18 + i * 0.01));
    return { loop: d.seamless(base, 0.6) };
  };

  return {
    sounds: {
      boot: d.room(d.mix([
        [d.seq(['C4', 'G4', 'D5', 'E5', 'B5'], 0.09, (f) => marimba(f, 1.8)), 0, 0.7],
        [pad(2.2, ['C4', 'G4', 'E5'], 4.4), 0.1, 0.25]
      ]), 0.6, 0.5, 0.25, 1),
      'rare-boot': d.room(d.mix([
        [d.seq(['C4', 'G4', 'D5', 'E5', 'B5', 'D6', 'G6'], 0.08, (f) => marimba(f, 1.8)), 0, 0.65],
        [pad(2.6, ['C4', 'G4', 'B4', 'E5'], 5.2), 0.1, 0.25]
      ]), 0.7, 0.5, 0.3, 1.2),
      play: d.mix([[tap(520), 0], [tap(780, 0.1), 0.045, 0.7]]),
      pause: d.mix([[tap(780, 0.1), 0], [tap(520), 0.045, 0.7]]),
      skip: d.mix([[tap(660, 0.08), 0], [tap(880, 0.08), 0.06, 0.8]]),
      seek: tap(990, 0.06),
      buffering: buffering(),
      'long-wait': longMelody(),
      found: d.mix([[marimba(d.hz('E5')), 0], [marimba(d.hz('B5')), 0.08, 0.8]]),
      stalled: d.mix([[marimba(d.hz('G4')), 0], [marimba(d.hz('D4')), 0.12, 0.8]]),
      saved: d.room(d.seq(['C5', 'E5', 'G5'], 0.07, (f) => marimba(f)), 0.4, 0.5, 0.2, 0.6),
      liked: d.room(d.mix([[marimba(d.hz('A5')), 0], [marimba(d.hz('E6')), 0.06, 0.7]]), 0.4, 0.5, 0.25, 0.6),
      queued: d.shape(d.osc(0.08, (t) => 900 - 400 * Math.min(1, t / 0.04)), d.perc(0.002, 0.05)),
      error: d.mix([
        [d.shape(d.osc(0.25, d.hz('D4'), d.tri), d.perc(0.004, 0.18)), 0, 0.8],
        [d.shape(d.osc(0.35, d.hz('A3'), d.tri), d.perc(0.004, 0.25)), 0.13, 0.8]
      ]),
      design: d.room(d.mix([
        [d.shape(d.biquad(d.noise(0.45, rng), 'bandpass', (t) => 600 + 5000 * t / 0.45, 2.5), d.arch(0.45)), 0, 0.5],
        [marimba(d.hz('G5')), 0.32, 0.6]
      ]), 0.5, 0.5, 0.25, 0.6),
      classic: d.seq(['E5', 'A5', 'E6'], 0.07, (f) => d.shape(d.osc(0.12, f, d.square(0.5, 0.04)), d.perc(0.002, 0.07))),
    },
    eggs: {
      'night-owl': d.room(d.mix([
        [d.shape(d.osc(0.5, (t) => 380 - 60 * t), d.arch(0.5)), 0, 0.6],
        [d.shape(d.osc(0.9, (t) => 400 - 90 * t), d.arch(0.9)), 0.65, 0.6]
      ]), 0.7, 0.6, 0.35, 1),
      'new-year': d.room(d.seq(['C5', 'E5', 'G5', 'C6', 'E6', 'G6', 'C7'], 0.07, (f) => marimba(f, 1.4)), 0.7, 0.4, 0.3, 1),
      birthday: d.room(d.seq(['G4', 'C5', 'E5', 'D5', 'C5', 'G5'], 0.15, (f) => marimba(f, 1.2)), 0.5, 0.5, 0.25, 0.8),
      milestone: d.room(d.mix([
        [d.seq(['C5', 'G5', 'C6', 'E6'], 0.09, (f) => marimba(f, 1.4)), 0],
        [d.seq(['E5', 'C6', 'G6'], 0.09, (f) => marimba(f, 1.4)), 0.4, 0.6]
      ]), 0.6, 0.4, 0.3, 1),
    }
  };
}
