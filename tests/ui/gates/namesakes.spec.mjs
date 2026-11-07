// ADR-0035's name page, with the `namesakes` switch on: every artist called the name, told apart by their own line,
// the listener's first; their songs nothing places; and the listener's word, which nothing picks for them.

import { expect, test } from '@playwright/test';
import { ownHub, settle } from './lib.mjs';

let hub;
test.beforeAll(async () => { hub = await ownHub({ LULL_FLAGS: 'namesakes' }); });
test.afterAll(() => hub?.close());

test('the artists of a shared name are told apart, and the listener says whose a song is', async ({ page }) => {
  await page.goto(`${hub.url}/#/artists/named/Soft%20Choir`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await settle(page);
  const rows = page.locator('.lib-named .lib-card').first().locator('.lib-frow');
  await expect(rows).toHaveCount(3);
  await expect(rows.first()).toContainText('Yours');
  await expect(rows.nth(1)).toContainText('1970s Welsh male voice choir');
  await expect(rows.nth(2)).toContainText('Group · NZ · since 2019');

  await page.locator('[data-act="ns-whose"]').first().click();
  const picks = page.locator('.ns-pick input[name="ns-pick"]');
  await expect(picks).toHaveCount(4);
  await expect(page.locator('.ns-pick input[name="ns-pick"]:checked')).toHaveCount(0);   // nothing is picked for them
  await page.locator('[data-act="ns-say"]').click();
  await expect(page.locator('.ns-pick')).toBeVisible();                                    // Save asks them to pick first

  await picks.last().check();
  const said = page.waitForRequest((r) => r.url().endsWith('/api/artists/named/split') && r.method() === 'POST');
  await page.locator('[data-act="ns-say"]').click();
  const body = (await said).postDataJSON();
  expect(body.name).toBe('Soft Choir');
  expect(body.tracks).toHaveLength(1);
});
