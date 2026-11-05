// A theme's words are as long as its author wrote them: the picker card clips a long description, Your themes shows it
// whole, and Theme Studio only hints that the card clips it. The hub refuses only text past its safety bounds
// (crates/themes TEXT_LIMITS, which Designs.limits mirrors), and Studio won't save or export that, saying which field
// and by how much. A theme in Studio is signed by whoever makes it, never by lull. On a fixture hub of the gate's own.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openScreen, ownHub } from './lib.mjs';

// a friend's theme begun from a copy of a built-in design: lull's name as author, two full sentences of description
const LONG = 'A sunny desktop from long ago, with pixel clouds, candy windows and title bars, and a clear deck with its wires showing. '
  + 'By night the same desktop under a bright moon, in darker windows, the deck lit from behind, every wire aglow.';

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-theme-text-'));
  mkdirSync(join(dir, 'candy'));
  writeFileSync(join(dir, 'candy', 'theme.json'), JSON.stringify({ id: 'candy', name: 'Candy', author: 'lull', api: 1, extends: null, shell: 'rail', swatches: ['#f8b2d3', '#fff3f9', '#e0337f', '#2a1031'], modes: ['day', 'night'], description: LONG, slots: [] }));
  writeFileSync(join(dir, 'candy', 'theme.css'), '');
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

async function appearance(browser) {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await openScreen(page, '/settings/appearance', { clock: false });
  return page;
}
test.afterEach(async ({ browser }) => { for (const c of browser.contexts()) await c.close(); });
const saving = (page, id) => page.waitForRequest((r) => r.method() === 'PUT' && r.url().endsWith(`/api/themes/${id}/theme.json`));

test('a long description is clipped on the card, shown whole in Your themes, and only hinted at in Theme Studio', async ({ browser }) => {
  expect(LONG.length).toBeGreaterThan(200);
  const page = await appearance(browser);
  const card = page.locator('.sys-design[data-value="candy"] .sys-design__desc');
  await expect(card).toHaveText(LONG);
  expect(await card.evaluate((el) => el.scrollHeight > el.clientHeight + 1), 'the card clips it').toBe(true);
  await expect(page.locator('.sys-themes__desc')).toHaveText(LONG);

  await page.locator('[data-act="theme-edit"][data-id="candy"]').click();
  await expect(page.locator('[data-studio="description"]')).toHaveValue(LONG);
  await expect(page.locator('[data-studio-hint]')).toBeVisible();
  await expect(page.locator('[data-studio="description"]')).toHaveAttribute('aria-invalid', 'false');
  await expect(page.locator('[data-studio-count="description"]')).toBeHidden();
  await expect(page.locator('[data-studio="author"]'), 'a copy of a built-in is the listener\'s to sign').toHaveValue('');

  const saved = saving(page, 'candy');
  await page.locator('[data-studio-act="save"]').click();
  const m = JSON.parse((await saved).postData());
  expect(m.description).toBe(LONG);
  expect(m.author).toBe('');
});

test('a new theme in Theme Studio is signed by its maker, never by lull', async ({ browser }) => {
  const page = await appearance(browser);
  await page.locator('[data-act="theme-studio"]').click();
  await expect(page.locator('[data-studio="author"]')).toHaveValue('');
  await expect(page.locator('[data-studio="author"]')).toHaveAttribute('placeholder', 'Your name');
  let saved = saving(page, 'my-lull');
  await page.locator('[data-studio-act="save"]').click();
  expect(JSON.parse((await saved).postData()).author).toBe('');
  await page.locator('[data-studio="author"]').fill('Sam');
  saved = saving(page, 'my-lull');
  await page.locator('[data-studio-act="save"]').click();
  expect(JSON.parse((await saved).postData()).author).toBe('Sam');
});

test('Theme Studio will not save or export a description past the hub\'s bound, and says which and by how much', async ({ browser }) => {
  const page = await appearance(browser);
  const limit = await page.evaluate(() => window.Designs.limits.description);
  await page.locator('[data-act="theme-edit"][data-id="candy"]').click();
  const desc = page.locator('[data-studio="description"]'), count = page.locator('[data-studio-count="description"]');
  const writes = [];
  page.on('request', (r) => { if (r.method() !== 'GET' && r.url().includes('/api/themes')) writes.push(r.url()); });
  page.on('download', (d) => writes.push(d.url()));
  await desc.fill('a'.repeat(limit + 1));
  await expect(count).toHaveText(`${limit + 1} / ${limit}`);
  await expect(desc).toHaveAttribute('aria-invalid', 'true');
  await expect(page.locator('[data-studio-hint]')).toBeHidden();
  for (const act of ['export', 'save']) {
    await page.locator(`[data-studio-act="${act}"]`).click();
    await expect(page.locator('#toasts')).toContainText(`Description is ${limit + 1} characters, at most ${limit}.`);
    await expect(desc).toBeFocused();
  }
  expect(writes, 'nothing past the bound reaches the hub or a file').toEqual([]);
});
