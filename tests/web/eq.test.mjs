// The equaliser's settings and curve (web/eq.js): what Bass, Treble and the presets do to the bands, and the
// automatic level that keeps a boost from clipping.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const EQ = createRequire(import.meta.url)('../../web/eq.js');
const AUDIBLE = Array.from({ length: 400 }, (_, i) => 20 * 1000 ** (i / 399));
const peak = (s) => Math.max(...EQ.response(s, AUDIBLE));

test('a flat equaliser does nothing and needs no filters', () => {
  const s = EQ.flat();
  assert.equal(EQ.active(s), false);
  assert.equal(EQ.level(s), 0);
  assert.ok(Math.abs(peak(s)) < 1e-9);
  assert.equal(EQ.preset(s), 'flat');
});

test('a boost is levelled down so the loudest frequency stays under 0 dB', () => {
  for (const p of EQ.PRESETS) {
    const s = EQ.withPreset(EQ.flat(), p.id);
    const loudest = peak(s) + EQ.level(s);
    assert.ok(loudest <= 0, `${p.name} peaks at ${loudest.toFixed(2)} dB after its level`);
    assert.ok(loudest > -0.5, `${p.name} is levelled ${(-loudest).toFixed(2)} dB more than it needs`);
  }
  const most = EQ.withBass(EQ.withTreble(EQ.flat(), 12), 12);
  assert.ok(peak(most) + EQ.level(most) <= 0);
});

test('a cut is never made louder to make up for it', () => {
  const s = EQ.withPreset(EQ.flat(), 'less-bass');
  assert.equal(EQ.level(s), 0);
});

test('a hand-set level (Webamp\'s preamp) is kept until Bass, Treble or a preset is moved', () => {
  let s = EQ.withPreamp(EQ.withPreset(EQ.flat(), 'bass'), -2);
  assert.equal(EQ.level(s), -2);
  s = EQ.withBand(s, 4, 3);
  assert.equal(EQ.level(s), -2);
  s = EQ.withBass(s, 3);
  assert.equal(s.preamp, null);
  assert.ok(EQ.level(s) < 0);
});

test('Bass and Treble read back what was set, and keep the rest of a hand-drawn curve', () => {
  let s = EQ.withBass(EQ.flat(), 4);
  assert.equal(EQ.bass(s), 4);
  assert.equal(EQ.treble(s), 0);
  s = EQ.withTreble(s, -3);
  assert.equal(EQ.bass(s), 4);
  assert.equal(EQ.treble(s), -3);
  assert.equal(s.bands[4], 0, '1 kHz is neither bass nor treble');

  const drawn = EQ.withBand(EQ.flat(), 1, 5);
  const lifted = EQ.withBass(drawn, EQ.bass(drawn) + 2);
  assert.ok(lifted.bands[1] > lifted.bands[2], 'the 170 Hz bump is still there');
});

test('the presets are named shapes the bands can be recognised as', () => {
  for (const p of EQ.PRESETS) assert.equal(EQ.preset(EQ.withPreset(EQ.flat(), p.id)), p.id);
  assert.equal(EQ.preset(EQ.withBand(EQ.flat(), 4, 2)), null);
  assert.ok(EQ.bass(EQ.withPreset(EQ.flat(), 'bass')) > 0);
});

test('anything stored that is not a whole equaliser reads as flat, and values stay within ±12 dB', () => {
  assert.deepEqual(EQ.normalise(null), EQ.flat());
  assert.deepEqual(EQ.normalise({ bands: [1, 2] }), EQ.flat());
  const s = EQ.normalise({ on: true, bands: [40, -40, 0, 0, 0, 0, 0, 0, 0, 'x'], preamp: 99 });
  assert.equal(s.bands[0], 12);
  assert.equal(s.bands[1], -12);
  assert.equal(s.bands[9], 0);
  assert.equal(s.preamp, 12);
});

test('turned off, nothing runs', () => {
  const s = { ...EQ.withPreset(EQ.flat(), 'bass'), on: false };
  assert.equal(EQ.active(s), false);
});
