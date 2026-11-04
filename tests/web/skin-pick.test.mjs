// Random skin's rule (web/desktop/webamp-session.js LullWebamp.pickSkin), on the cases in tests/skin-pick.json that any
// other player's copy of the rule shares.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const W = createRequire(import.meta.url)('../../web/desktop/webamp-session.js');
const { cases } = JSON.parse(readFileSync(new URL('../skin-pick.json', import.meta.url), 'utf8'));
const skins = (md5s) => md5s.map((md5) => ({ md5, name: md5.toUpperCase() }));

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
