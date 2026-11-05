// The sound rules (web/sound-rules.js, ADR-0081): never over the music, never while lull isn't in front, quiet when the
// system asks, and the packs stacked as the ADR says. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const require = createRequire(import.meta.url);
const S = require('../../web/sound-rules.js');
const vocab = JSON.parse(readFileSync(new URL('../../web/packs/vocabulary.json', import.meta.url), 'utf8'));
const on = S.settings({});
const ctx = (o) => Object.assign({ heard: false, front: true, quiet: false, reducedMotion: false, classic: false }, o);

test('while music plays only play and pause sound, and the buffering loop never does', () => {
  assert.equal(S.allowed('play', ctx({ heard: true }), on, vocab), true);
  assert.equal(S.allowed('pause', ctx({ heard: true }), on, vocab), true);
  for (const e of ['skip', 'liked', 'queued', 'found', 'error', 'boot', 'buffering']) {
    assert.equal(S.allowed(e, ctx({ heard: true }), on, vocab), false, e);
  }
  const overMusic = S.settings({ onlySilent: false });
  assert.equal(S.allowed('liked', ctx({ heard: true }), overMusic, vocab), true);
  assert.equal(S.allowed('buffering', ctx({ heard: true }), overMusic, vocab), false);
  assert.equal(S.allowed('liked', ctx({ heard: true, egg: true }), on, vocab), false);
});

test('nothing sounds while lull is not in front, except the background sounds the listener asked for', () => {
  assert.equal(S.allowed('found', ctx({ front: false }), on, vocab), false);
  const asked = S.settings({ background: { found: true, liked: true } });
  assert.equal(S.allowed('found', ctx({ front: false }), asked, vocab), true);
  assert.equal(S.allowed('liked', ctx({ front: false }), asked, vocab), false, 'liked is not a background sound');
});

test('the system asking for quiet or reduced motion silences everything, unless the listener says otherwise', () => {
  assert.equal(S.allowed('play', ctx({ quiet: true }), on, vocab), false);
  assert.equal(S.allowed('play', ctx({ reducedMotion: true }), on, vocab), false);
  assert.equal(S.allowed('play', ctx({ quiet: true }), S.settings({ followQuiet: false }), vocab), true);
});

test('switches: the master switch, one sound, a sound off by default, Classic resting, the surprises', () => {
  assert.equal(S.allowed('play', ctx(), S.settings({ on: false }), vocab), false);
  assert.equal(S.allowed('play', ctx(), S.settings({ sounds: { play: false } }), vocab), false);
  assert.equal(S.allowed('seek', ctx(), on, vocab), false);
  assert.equal(S.allowed('seek', ctx(), S.settings({ sounds: { seek: true } }), vocab), true);
  assert.equal(S.allowed('play', ctx({ classic: true }), on, vocab), false);
  assert.equal(S.allowed('classic', ctx({ classic: true }), on, vocab), true);
  assert.equal(S.allowed('moon', ctx({ egg: true }), S.settings({ surprises: false }), vocab), false);
  assert.equal(S.allowed('moon', ctx({ egg: true }), S.settings({ eggsOff: { moon: true } }), vocab), false);
  assert.equal(S.allowed('moon', ctx({ egg: true }), on, vocab), true);
});

test('stored settings that are broken fall back to the defaults', () => {
  const s = S.settings({ on: 'yes', volume: 400, packs: ['a', 3], sounds: [], extra: 1 });
  assert.equal(s.on, true);
  assert.equal(s.volume, 100);
  assert.deepEqual(s.packs, ['a']);
  assert.deepEqual(s.sounds, {});
  assert.equal('extra' in s, false);
});

const known = {
  default: { id: 'default', sounds: { play: 'd/play.ogg', skip: 'd/skip.ogg', liked: 'd/liked.ogg' }, eggs: [{ name: 'moon', when: [], sound: 'd/moon.ogg' }, { name: 'sun', when: [] }] },
  petal: { id: 'petal', sounds: { play: ['p/play-1.ogg', 'p/play-2.ogg'], liked: null }, eggs: [{ name: 'moon', when: [], sound: 'p/moon.ogg' }] },
  friend: { id: 'friend', sounds: { skip: { url: 'f/skip.ogg' } }, eggs: [{ name: 'rainy', when: [] }] }
};

test('a sound comes from the first pack on that has it; null silences it on purpose', () => {
  const stack = S.stack(S.settings({ packs: ['friend'] }), 'petal', known);
  assert.deepEqual(stack.map((p) => p.id), ['friend', 'petal', 'default']);
  assert.deepEqual(S.soundFor('skip', stack, on, known).entry.urls, ['f/skip.ogg']);
  assert.deepEqual(S.soundFor('play', stack, on, known).entry.urls, ['p/play-1.ogg', 'p/play-2.ogg']);
  assert.equal(S.soundFor('liked', stack, on, known), null);
  assert.equal(S.pick(S.soundFor('play', stack, on, known).entry, 3), 'p/play-2.ogg');
});

test('a pack switched off, or unknown here, is left out; default comes last once', () => {
  const s = S.settings({ packs: ['default', 'gone', 'friend'], packsOff: { friend: true } });
  assert.deepEqual(S.stack(s, 'petal', known).map((p) => p.id), ['default', 'petal']);
});

test('one sound replaced by the listener wins over every pack', () => {
  const s = S.settings({ replace: { play: { pack: 'friend', sound: 'skip' } } });
  assert.deepEqual(S.soundFor('play', S.stack(s, 'petal', known), s, known).entry.urls, ['f/skip.ogg']);
});

test('eggs of every pack on are merged by name, the higher pack winning, minus those switched off', () => {
  const stack = S.stack(S.settings({ packs: ['friend'] }), 'petal', known);
  const eggs = S.eggs(stack, on);
  assert.deepEqual(Object.keys(eggs).sort(), ['moon', 'rainy', 'sun']);
  assert.equal(eggs.moon.sound, 'p/moon.ogg');
  assert.equal(eggs.moon.pack, 'petal');
  assert.deepEqual(S.soundFor('moon', stack, on, known, true).entry.urls, ['p/moon.ogg']);
  assert.equal(S.soundFor('sun', stack, on, known, true), null);
  assert.equal('moon' in S.eggs(stack, S.settings({ eggsOff: { moon: true } })), false);
});

test("the look's pack is the theme's own, else the design it is or extends", () => {
  const built = ['default', 'skydeck'];
  assert.equal(S.lookPack('rainy-day', 'skydeck', ['rainy-day'], built), 'theme:rainy-day');
  assert.equal(S.lookPack('paper-moon', 'skydeck', [], built), 'skydeck');
  assert.equal(S.lookPack('bare', null, [], built), null);
});

test('levelling: loud sounds come down to the same peak and loudness, quiet ones are never made louder', () => {
  const rate = 48000, loud = new Float32Array(rate).map((_, i) => 0.9 * Math.sin(i / 10));
  const g = S.levelGain(loud, rate);
  assert.ok(Math.abs(0.9 * g - S.PEAK) < 1e-9 || 0.9 * g / Math.SQRT2 <= S.RMS + 1e-9);
  assert.ok(0.9 * g <= S.PEAK + 1e-9);
  const quiet = new Float32Array(rate).map((_, i) => 0.01 * Math.sin(i / 10));
  assert.equal(S.levelGain(quiet, rate), 1);
  assert.equal(S.levelGain(new Float32Array(100), rate), 0);
});
