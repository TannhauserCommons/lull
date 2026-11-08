// The colour along lull's Discord card (src/colour.js): read from a cover's JPEG without decoding it all, or from its
// PNG, the same whatever flavour the cover comes in, and none when it isn't a picture this reads.
// Run: npm test (in share/)
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { coverColour } from '../src/colour.js';
import { CLEAR_PNG, GREY, PNG, SOLID, TWO_BASELINE, TWO_PALETTE, TWO_PNG, TWO_PROGRESSIVE, TWO_RESTARTS } from './pictures.mjs';

const bytes = (b64) => new Uint8Array(Buffer.from(b64, 'base64'));
const colour = async (b64) => {
  const hex = await coverColour(bytes(b64));
  assert.ok(hex, 'a colour');
  return [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
};
const near = (got, want, by) => got.every((v, i) => Math.abs(v - want[i]) <= by);

test('a cover of one colour that reads on any theme is its own colour', async () => {
  const got = await colour(SOLID);
  assert.ok(near(got, [52, 110, 190], 4), `${got} is (52, 110, 190)`);
});

test('a cover\'s colour is its most colourful area, not the average of everything on it', async () => {
  // Three quarters pale cream, one quarter red: the average is a dull pink; the card takes the red.
  const [r, g, b] = await colour(TWO_BASELINE);
  assert.ok(r > g + 90 && r > b + 80, `${[r, g, b]} is red`);
});

test('progressive JPEGs and restart markers give the same colour as a baseline JPEG', async () => {
  const base = await colour(TWO_BASELINE);
  for (const [name, other] of Object.entries({ TWO_PROGRESSIVE, TWO_RESTARTS })) {
    const got = await colour(other);
    assert.ok(near(got, base, 10), `${name}: ${got} is ${base}`);
  }
});

test('a PNG, RGB or palette, gives its colour exactly (a JPEG\'s halved colour resolution blurs it towards the rest)', async () => {
  for (const [name, png] of Object.entries({ TWO_PNG, TWO_PALETTE })) {
    const got = await colour(png);
    assert.ok(near(got, [200, 40, 50], 6), `${name}: ${got} is (200, 40, 50)`);
  }
});

test('a PNG\'s clear pixels don\'t count', async () => {
  const got = await colour(CLEAR_PNG);
  assert.ok(near(got, [52, 110, 190], 2), `${got} is (52, 110, 190), not green`);
  const [r, g, b] = await colour(PNG);
  assert.ok(r > g + 90 && r > b + 80, `${[r, g, b]} is red`);
});

test('a black and white cover gives a grey, lightened enough to see on a dark theme', async () => {
  const [r, g, b] = await colour(GREY);
  assert.ok(r === g && g === b, `${[r, g, b]} is grey`);
  assert.ok(r >= 90, 'the shade (90) is lifted, not darker');
});

test('anything that isn\'t a picture this reads has no colour, and never throws', async () => {
  assert.equal(await coverColour(bytes(TWO_BASELINE).subarray(0, 300)), null, 'a JPEG cut short');
  assert.equal(await coverColour(bytes(TWO_PNG).subarray(0, 60)), null, 'a PNG cut short');
  const png = bytes(TWO_PNG);
  const broken = png.slice();
  broken.fill(0x55, 50, png.length - 20);
  assert.equal(await coverColour(broken), null, 'a PNG whose picture doesn\'t inflate');
  assert.equal(await coverColour(new Uint8Array(0)), null);
  assert.equal(await coverColour(new TextEncoder().encode('picture:itunes')), null);
});
