// lull's built-in packs (web/packs, ADR-0081): their source still renders the sounds that were encoded, and the page's
// lists agree with the one vocabulary. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { check, PACKS } from '../../scripts/sounds/render.mjs';

const vocab = JSON.parse(readFileSync(new URL('../../web/packs/vocabulary.json', import.meta.url), 'utf8'));

test("every built-in pack is rendered from its current source", async () => {
  assert.deepEqual(await check(), []);
});

test("lull's own eggs use only the vocabulary's conditions, and every design pack has every sound event", () => {
  for (const id of PACKS) {
    const pack = JSON.parse(readFileSync(new URL(`../../web/packs/${id}/pack.json`, import.meta.url), 'utf8'));
    assert.deepEqual(Object.keys(pack.sounds), vocab.events, id);
    for (const egg of pack.eggs) {
      for (const c of egg.when) assert.ok(vocab.conditions[Object.keys(c)[0]], `${id}/${egg.name}`);
      if (egg.once) assert.ok(vocab.once.includes(egg.once), `${id}/${egg.name}`);
    }
  }
});
