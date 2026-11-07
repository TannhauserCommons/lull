// The colour along lull's Discord card (src/colour.js): read from a cover's JPEG without decoding it all, the same
// whatever JPEG flavour the cover comes in, and none when it isn't a JPEG.
// Run: npm test (in share/)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { coverColour } from '../src/colour.js';
import { GREY, PNG, SOLID, TWO_BASELINE, TWO_PROGRESSIVE, TWO_RESTARTS } from './pictures.mjs';

const bytes = (b64) => new Uint8Array(Buffer.from(b64, 'base64'));
const rgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));

test('a cover of one colour that reads on any theme is its own colour', () => {
  const [r, g, b] = rgb(coverColour(bytes(SOLID)));
  for (const [got, want] of [[r, 52], [g, 110], [b, 190]]) assert.ok(Math.abs(got - want) <= 4, `${[r, g, b]} is (52, 110, 190)`);
});

test('a cover\'s colour is its most colourful area, not the average of everything on it', () => {
  // Three quarters pale cream, one quarter red: the average is a dull pink; the card takes the red.
  const [r, g, b] = rgb(coverColour(bytes(TWO_BASELINE)));
  assert.ok(r > g + 90 && r > b + 80, `${[r, g, b]} is red`);
});

test('progressive JPEGs and JPEGs with restart markers give the same colour as a baseline one', () => {
  const base = rgb(coverColour(bytes(TWO_BASELINE)));
  for (const other of [TWO_PROGRESSIVE, TWO_RESTARTS]) {
    const got = rgb(coverColour(bytes(other)));
    assert.ok(got.every((v, i) => Math.abs(v - base[i]) <= 8), `${got} is ${base}`);
  }
});

test('a black and white cover gives a grey, lightened enough to see on a dark theme', () => {
  const [r, g, b] = rgb(coverColour(bytes(GREY)));
  assert.ok(r === g && g === b, `${[r, g, b]} is grey`);
  assert.ok(r >= 90, 'the shade (90) is lifted, not darker');
});

test('anything that isn\'t a JPEG this reads has no colour, and never throws', () => {
  assert.equal(coverColour(bytes(PNG)), null);
  const jpeg = bytes(TWO_BASELINE);
  assert.equal(coverColour(jpeg.subarray(0, 300)), null, 'cut short');
  assert.equal(coverColour(new Uint8Array(0)), null);
  assert.equal(coverColour(new TextEncoder().encode('picture:itunes')), null);
});
