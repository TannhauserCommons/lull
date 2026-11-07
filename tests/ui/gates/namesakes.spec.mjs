// ADR-0035's name page, with the `namesakes` switch on: every artist called the name, told apart by their own line,
// the listener's first; their songs nothing places; a follow from before, asked about; and the listener's word,
// which nothing picks for them.

import { expect, test } from '@playwright/test';
import { ownHub, settle } from './lib.mjs';

let hub;
test.beforeAll(async () => { hub = await ownHub({ LULL_FLAGS: 'namesakes' }); });
test.afterAll(() => hub?.close());

test('the artists of a shared name are told apart, and the listener says whose a song is', async ({ page }) => {
  await page.goto(`${hub.url}/#/artists/named/Soft%20Choir`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await settle(page);
  // A follow from before that nothing says which of: asked, one button each.
  const ask = page.locator('[data-act="ns-follow-this"]');
  await expect(ask).toHaveCount(3);
  const rows = page.locator('[aria-labelledby="ns-list-h"] .lib-frow');
  await expect(rows).toHaveCount(3);
  await expect(rows.first()).toContainText('Yours');
  await expect(rows.nth(1)).toContainText('1970s Welsh male voice choir');
  await expect(rows.nth(2)).toContainText('Group · NZ · since 2019');

  const chose = page.waitForRequest((r) => r.url().endsWith('/api/artists/named/choose') && r.method() === 'POST');
  await ask.nth(1).click();
  expect((await chose).postDataJSON()).toEqual({ name: 'Soft Choir', artist: 'ar-5c0f7c401c0a0001' });

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

test('artists sharing a name are separate search results, each saying which one it is', async ({ page }) => {
  await page.goto(`${hub.url}/#/search?q=soft%20choir`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await settle(page);
  const by = page.locator('.disc-shelf--artists .tile__by');
  await expect(by).toHaveCount(3);
  await expect(by.nth(1)).toHaveAttribute('data-tip', '1970s Welsh male voice choir');
  await expect(page.locator('.disc-top__by')).toHaveText('1990s English dream pop band');
});
