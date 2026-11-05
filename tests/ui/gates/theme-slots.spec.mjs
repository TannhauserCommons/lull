// A listener's theme shows its slot templates however it reached the hub: unzipped into the themes folder by hand, its
// theme.json lists no slots (the hub lists the templates its folder has, crates/themes store.rs), and a device that
// kept an older copy of the manifest, with no slots, loads the theme again once the hub's says otherwise. On a fixture
// hub of the gate's own: it serves a theme's manifest as the hub does.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openScreen, ownHub } from './lib.mjs';

const MANIFEST = { id: 'wired', name: 'Wired', api: 1, extends: null, shell: 'rail', swatches: ['#141516', '#d6d3c9', '#ffa53d', '#ffb24a'], modes: ['day', 'night'] };

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-theme-slots-'));
  mkdirSync(join(dir, 'wired', 'slots'), { recursive: true });
  writeFileSync(join(dir, 'wired', 'theme.json'), JSON.stringify(MANIFEST));          // no "slots", as an author writes it
  writeFileSync(join(dir, 'wired', 'theme.css'), '');
  writeFileSync(join(dir, 'wired', 'slots', 'brand.html'), '<span class="wordmark wd-mark">{{product}}<span class="wd-mark__tag">wd·01</span></span>');
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

test('a theme put in the folder by hand shows its slots, and a device\'s copy of its manifest without them catches up', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  const served = await (await page.request.get('/api/themes/wired/theme.json')).json();
  expect(served.slots, 'the hub lists the templates the folder has').toEqual(['brand']);

  // this device last saw the theme before its slots were listed
  await page.addInitScript((m) => localStorage.setItem('lull:themes', JSON.stringify({ wired: { ...m, slots: [] } })), MANIFEST);
  await openScreen(page, '/', { design: 'wired', clock: false });
  await expect(page.locator('.wd-mark__tag').first()).toHaveText('wd·01');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('lull:themes')).wired.slots)).toEqual(['brand']);
  await context.close();
});
