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

// every service notice behaves as the cache's does: closed, it stays closed through reloads until its cause has ended
const CAUSES = [
  ['sys-soulseek', 'soulseek', { state: 'down', detail: 'Not connected' }],
  ['sys-jellyfin', 'library', { state: 'down', detail: 'Jellyfin does not answer' }],
  ['sys-catalog', 'catalog', { state: 'down', detail: 'both catalogs are out' }],
  ['sys-lastfm', 'lastfm', { state: 'down', detail: 'Last.fm is out', extra: { queued: 2 } }],
];
for (const [id, service, report] of CAUSES) {
  test(`the ${service} notice, closed, stays closed through reloads, and comes again after its cause ended`, async ({ page }) => {
    await page.route('**/health', async (route) => {
      const res = await route.fetch();
      const h = await res.json();
      h.services[service] = report;
      await route.fulfill({ response: res, json: h });
    });
    const banner = page.locator(`#banners [data-banner="${id}"]`);
    await openScreen(page, '/home');
    await expect(banner).toBeVisible();
    await banner.locator('[data-banner-close]').click();
    await expect(banner).toHaveCount(0);
    for (let i = 0; i < 2; i++) {
      await page.reload();
      await page.waitForFunction((s) => window.App && App.server && App.server.checked && App.store.get('services')[s].state === 'down', service);
      await page.waitForTimeout(300);
      await expect(banner, `after reload ${i + 1}`).toHaveCount(0);
    }
    await page.evaluate((s) => App.setService(s, { state: 'ok', detail: 'back' }), service);
    await page.evaluate(([s, r]) => App.setService(s, r), [service, report]);
    await expect(banner).toBeVisible();
  });
}
