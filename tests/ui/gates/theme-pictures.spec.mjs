// A theme's card in Settings > Appearance shows a picture of the theme (ADR-0080): lull draws it from Home in the theme,
// day and night, and the hub keeps it under the theme's rev. Made on the next Appearance visit for a theme that has
// none (a theme put in the folder, or one from before), right after an import, and again after a Theme Studio save,
// for what was saved. The five built-in designs keep the pictures they ship. On a fixture hub of the gate's own: it
// stores themes and pictures.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { crc32 } from 'node:zlib';
import { openScreen, ownHub } from './lib.mjs';

const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
// a theme with no picture of its own, on petal, seen through a veil no design has: green by day, near black at night
const veil = (day, night) => `body::after { content: ""; position: fixed; inset: 0; z-index: 99; pointer-events: none; background: ${day}; }
:root[data-theme="night"] body::after { background: ${night}; }
`;
const theme = (id, name) => ({
  'theme.json': JSON.stringify({ id, name, author: 'gate', api: 1, extends: 'petal', shell: 'topnav', swatches: ['#e8f4e0', '#ffffff', '#2f8f3a', '#173d1c'], modes: ['day', 'night'] }),
  'theme.css': veil('rgba(40, 200, 60, .6)', 'rgba(0, 12, 0, .85)'),
});

// a picture waits for the page to be idle; in WebKit beside Theme Studio's two live previews that takes a while
test.describe.configure({ timeout: 180_000 });
const DRAWN = 90_000;

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-theme-pictures-'));
  for (const [file, text] of Object.entries(theme('meadow', 'Meadow'))) {
    mkdirSync(join(dir, 'meadow'), { recursive: true });
    writeFileSync(join(dir, 'meadow', file), text);
  }
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });
/* Settings > Appearance on this gate's hub */
async function appearance(browser) {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await openScreen(page, '/settings/appearance', { clock: false });
  return page;
}
test.afterEach(async ({ browser }) => { for (const c of browser.contexts()) await c.close(); });

/* the card's made picture for each mode ({ day, night } URLs), or null while it shows none: its class and its style read
   in one go, from the same render (a save renders the card again, for the new rev, before that rev has a picture) */
function shown(page, id) {
  return page.locator(`.sys-design[data-value="${id}"]`).evaluate((card) => {
    if (!card.querySelector('.sys-design__pv').classList.contains('has-shot')) return null;
    const url = (mode) => card.style.getPropertyValue('--pv-' + mode).match(/url\("([^"]+)"\)/)[1];
    return { day: url('day'), night: url('night') };
  });
}
/* the card's picture once it is the made one, and (with `not`) a different one from that, within the time a draw takes */
async function madePictures(page, id, not) {
  let got = null;
  await expect.poll(async () => { got = await shown(page, id); return !!got && got.day !== not; }, { timeout: DRAWN }).toBe(true);
  return got;
}
/* a picture as the hub serves it: its size, its mean colour, and how much its lower third varies (the page drawn to its
   foot: a page laid out for a shorter window leaves flat background there) */
async function look(page, url) {
  return page.evaluate(async (u) => {
    const r = await fetch(u);
    const blob = await r.blob(), bmp = await createImageBitmap(blob);
    const c = Object.assign(document.createElement('canvas'), { width: bmp.width, height: bmp.height }), g = c.getContext('2d');
    g.drawImage(bmp, 0, 0);
    const d = g.getImageData(0, 0, bmp.width, bmp.height).data, mean = [0, 0, 0], low = [];
    for (let i = 0; i < d.length; i += 4) {
      for (let k = 0; k < 3; k++) mean[k] += d[i + k] / (d.length / 4);
      if (i / 4 / bmp.width >= bmp.height * 2 / 3) low.push(d[i] + d[i + 1] + d[i + 2]);
    }
    const avg = low.reduce((a, x) => a + x, 0) / low.length;
    const spread = Math.sqrt(low.reduce((a, x) => a + (x - avg) ** 2, 0) / low.length);
    return { type: blob.type, width: bmp.width, height: bmp.height, mean: mean.map(Math.round), lowSpread: Math.round(spread) };
  }, url);
}
const SPREAD = 12;
const green = (m) => m[1] - Math.max(m[0], m[2]);
const light = (m) => m[0] + m[1] + m[2];

test('a theme with no picture gets one of itself, day and night, while the five designs keep theirs', async ({ browser }) => {
  const page = await appearance(browser);
  const shot = await madePictures(page, 'meadow');
  expect(shot.day).toMatch(/\/api\/theme-pictures\/meadow\/[0-9a-f]{16}-day\.webp$/);
  const day = await look(page, shot.day), night = await look(page, shot.night);
  expect(day).toMatchObject({ type: 'image/webp', width: 320, height: 200 });
  expect(night).toMatchObject({ type: 'image/webp', width: 320, height: 200 });
  expect(green(day.mean), 'the day picture shows the theme\'s green').toBeGreaterThan(40);
  expect(light(night.mean), 'the night picture is the theme at night').toBeLessThan(light(day.mean) - 150);
  expect(day.lowSpread, 'Home is drawn down to the foot of the picture').toBeGreaterThan(SPREAD);

  for (const id of DESIGNS) {
    const style = await page.locator(`.sys-design[data-value="${id}"]`).getAttribute('style');
    expect(style).toContain(`/designs/${id}/assets/preview-day.webp`);
  }
  await expect(page.locator('iframe')).toHaveCount(0);
});

test('an imported theme gets its picture right after the import', async ({ browser }) => {
  const page = await appearance(browser);
  await page.locator('[data-theme-file]').setInputFiles({ name: 'clover.lulltheme', mimeType: 'application/zip', buffer: zip(theme('clover', 'Clover')) });
  const shot = await madePictures(page, 'clover');
  expect(green((await look(page, shot.day)).mean)).toBeGreaterThan(40);
});

test('a Theme Studio save draws the card again from what was saved', async ({ browser }) => {
  const page = await appearance(browser);
  const before = await madePictures(page, 'meadow');
  await page.locator('[data-act="theme-edit"][data-id="meadow"]').click();
  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill(veil('rgba(220, 40, 140, .6)', 'rgba(20, 0, 10, .85)'));
  await page.locator('[data-studio-act="save"]').click();
  const after = await look(page, (await madePictures(page, 'meadow', before.day)).day);
  expect(green(after.mean), 'the new picture shows the saved pink, not the old green').toBeLessThan(-40);
});

/* a .lulltheme: the files, stored (a zip without compression) */
function zip(files) {
  const local = [], central = [];
  let at = 0;
  for (const [name, text] of Object.entries(files)) {
    const data = Buffer.from(text), path = Buffer.from(name), crc = crc32(data);
    const head = Buffer.alloc(30);
    head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt32LE(crc, 14);
    head.writeUInt32LE(data.length, 18); head.writeUInt32LE(data.length, 22); head.writeUInt16LE(path.length, 26);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0); dir.writeUInt16LE(20, 4); dir.writeUInt16LE(20, 6); dir.writeUInt32LE(crc, 16);
    dir.writeUInt32LE(data.length, 20); dir.writeUInt32LE(data.length, 24); dir.writeUInt16LE(path.length, 28); dir.writeUInt32LE(at, 42);
    local.push(head, path, data); central.push(dir, path);
    at += 30 + path.length + data.length;
  }
  const size = central.reduce((n, b) => n + b.length, 0), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(central.length / 2, 8); end.writeUInt16LE(central.length / 2, 10);
  end.writeUInt32LE(size, 12); end.writeUInt32LE(at, 16);
  return Buffer.concat([...local, ...central, end]);
}
