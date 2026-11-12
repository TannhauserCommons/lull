// Service banners (web/screens/system.js syncBanners). A listener (2026-11-12): the cache notice "keeps appearing on
// screen, on phone and PC, even after I close it". A closing lasted as long as the page, and the notice came for any
// cache at its size. Now a closed notice stays closed on this device until its cause ends, and the cache has one only
// for a full disk.
import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

const DISK_FULL = { state: 'down', detail: 'The transfers disk is full: nothing new can stream until there is room', extra: { usedBytes: 9.9e9, capBytes: 1e10 } };
const AT_SIZE = { state: 'ok', detail: '9.9 GB of 10.0 GB used, oldest audio evicted first', extra: { usedBytes: 9.9e9, capBytes: 1e10 } };

/* the fixture hub's /health, with the cache reporting `cache` */
async function cacheReports(page, cache) {
  await page.unroute('**/health');
  await page.route('**/health', async (route) => {
    const res = await route.fetch();
    const h = await res.json();
    h.services.cache = cache;
    await route.fulfill({ response: res, json: h });
  });
}

const notice = (page) => page.locator('#banners [data-banner="sys-cache"]');

test('a closed notice stays closed through a reload, and comes again once its cause has ended', async ({ page }) => {
  await cacheReports(page, DISK_FULL);
  await openScreen(page, '/home');
  await expect(notice(page)).toBeVisible();
  await notice(page).locator('[data-banner-close]').click();
  await expect(notice(page)).toHaveCount(0);

  await page.reload();
  await page.waitForFunction(() => window.App && App.server && App.server.checked && App.store.get('services').cache.state === 'down');
  await expect(notice(page)).toHaveCount(0);

  // the disk has room again, then fills: that is news
  await page.evaluate(() => App.setService('cache', { state: 'ok', detail: 'room again' }));
  await page.evaluate(() => App.setService('cache', { state: 'down', detail: 'The transfers disk is full' }));
  await expect(notice(page)).toBeVisible();
});

test('a cache at its size, or held over it by what plays, gets no notice', async ({ page }) => {
  await cacheReports(page, AT_SIZE);
  await openScreen(page, '/home');
  await page.evaluate(() => App.setService('cache', { state: 'warn', detail: 'held over its size by music that is playing' }));
  await expect(page.locator('#banners [data-banner]')).toHaveCount(0);
});
