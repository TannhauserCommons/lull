// Random skin's rule (web/desktop/webamp-session.js LullWebamp.pickSkin; docs/context/skins.md#random-skin).
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const W = createRequire(import.meta.url)('../../web/desktop/webamp-session.js');
const skins = (md5s) => md5s.map((md5) => ({ md5, name: md5.toUpperCase() }));

// Each case: the listener's own skins and favourites (md5s), the one in use (null: Webamp's base skin), what random()
// returns, and the md5 picked (null: there is no other skin).
const cases = [
  { name: 'one of their own, never the one in use', own: ['a', 'b', 'c'], favourites: ['f'], current: 'a', random: 0, pick: 'b' },
  { name: 'random\'s top end stays in range', own: ['a', 'b', 'c'], favourites: [], current: 'a', random: 0.9999999, pick: 'c' },
  { name: 'the one in use is out of the draw, not skipped after it', own: ['a', 'b', 'c'], favourites: [], current: 'b', random: 0.5, pick: 'c' },
  { name: 'a single skin of their own, while another is in use', own: ['a'], favourites: ['f', 'g'], current: 'f', random: 0.7, pick: 'a' },
  { name: 'a single skin of their own, already in use: there is no other', own: ['a'], favourites: ['f', 'g'], current: 'a', random: 0.3, pick: null },
  { name: 'none of their own: the favourites', own: [], favourites: ['f', 'g'], current: null, random: 0.6, pick: 'g' },
  { name: 'none of their own, the only favourite in use: there is no other', own: [], favourites: ['f'], current: 'f', random: 0, pick: null },
  { name: 'nothing to pick from', own: [], favourites: [], current: null, random: 0.5, pick: null },
];

for (const c of cases) {
  test(c.name, () => {
    const s = W.pickSkin(skins(c.own), skins(c.favourites), c.current, () => c.random);
    assert.equal(s ? s.md5 : null, c.pick);
  });
}

test('Random never returns the skin in use, whatever the draw', () => {
  for (let i = 0; i < 1000; i += 1) {
    const s = W.pickSkin(skins(['a', 'b']), [], 'a');
    assert.equal(s.md5, 'b');
  }
});

test('every other skin can come up', () => {
  const seen = new Set();
  for (let i = 0; i < 2000; i += 1) seen.add(W.pickSkin(skins(['a', 'b', 'c', 'd']), [], 'c').md5);
  assert.deepEqual([...seen].sort(), ['a', 'b', 'd']);
});
