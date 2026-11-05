// A theme's name, author and description are at most as long as the hub takes (crates/themes TEXT_LIMITS, which
// Designs.limits mirrors): Theme Studio counts each while you type and refuses to save or export past it, saying which
// and by how much, so a theme Studio makes always imports. On a fixture hub of the gate's own with one theme.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openScreen, ownHub } from './lib.mjs';

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-theme-text-'));
  mkdirSync(join(dir, 'meadow'));
  writeFileSync(join(dir, 'meadow', 'theme.json'), JSON.stringify({ id: 'meadow', name: 'Meadow', author: 'gate', api: 1, extends: 'petal', shell: 'topnav', swatches: ['#e8f4e0', '#ffffff', '#2f8f3a', '#173d1c'], modes: ['day', 'night'], description: 'Green.' }));
  writeFileSync(join(dir, 'meadow', 'theme.css'), '');
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

test('Theme Studio counts a description against the limit and will not save or export one past it', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await openScreen(page, '/settings/appearance', { clock: false });
  const limit = await page.evaluate(() => window.Designs.limits.description);
  await page.locator('[data-act="theme-edit"][data-id="meadow"]').click();
  const desc = page.locator('[data-studio="description"]'), count = page.locator('[data-studio-count="description"]');
  await expect(count).toHaveText(`6 / ${limit}`);

  const writes = [];
  page.on('request', (r) => { if (r.method() !== 'GET' && r.url().includes('/api/themes')) writes.push(r.url()); });
  page.on('download', (d) => writes.push(d.url()));
  await desc.fill('a'.repeat(limit + 1));
  await expect(count).toHaveText(`${limit + 1} / ${limit}`);
  await expect(count).toHaveClass(/is-over/);
  await expect(desc).toHaveAttribute('aria-invalid', 'true');
  for (const act of ['export', 'save']) {
    await page.locator(`[data-studio-act="${act}"]`).click();
    await expect(page.locator('#toasts')).toContainText(`Description is ${limit + 1} characters, at most ${limit}.`);
    await expect(desc).toBeFocused();
  }
  expect(writes, 'nothing past the limit reaches the hub or a file').toEqual([]);

  await desc.fill('a'.repeat(limit));
  await expect(count).not.toHaveClass(/is-over/);
  await expect(desc).toHaveAttribute('aria-invalid', 'false');
  const saved = page.waitForRequest((r) => r.method() === 'PUT' && r.url().endsWith('/api/themes/meadow/theme.json'));
  await page.locator('[data-studio-act="save"]').click();
  expect(JSON.parse((await saved).postData()).description).toHaveLength(limit);
  await context.close();
});
