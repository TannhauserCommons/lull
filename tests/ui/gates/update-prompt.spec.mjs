// The "New version ready" note (web/screens/system.js, app.js loadHealth and registerWorker). A listener (2026-11-13):
// it kept coming back after Reload, many times. One Reload offered the update, installed nothing and reloaded; the
// reload then found the worker for the new version waiting and offered the same update again. Now the note is offered
// once when the hub's version changes under an open page, one Reload finishes it, and nothing else offers it.
import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

test.use({ serviceWorkers: 'allow' });

const V1 = '1.0.0+1.aaaaaaa', V2 = '1.0.0+2.bbbbbbb';
/* the hub says it is `v` (or, with `v` null, answers /health with no version: a proxy's error page, a hub starting) */
async function hubIs(page, v) {
  await page.unroute('**/health');
  await page.route('**/health', async (route) => {
    let res;
    try { res = await route.fetch(); } catch { await route.abort('internetdisconnected'); return; }   // offline: the hub can't be reached
    const h = await res.json();
    if (v) h.version = v; else delete h.version;
    await route.fulfill({ response: res, json: h });
  });
}
const note = (page) => page.locator('#toasts .toast', { hasText: 'New version' });
const running = (page) => page.evaluate(() => navigator.serviceWorker.getRegistration().then((r) => (r.active ? new URL(r.active.scriptURL).searchParams.get('v') : null)));
async function started(page) {
  await page.waitForFunction(() => window.App && App.server && App.server.checked && App.server.version);
  await page.evaluate(() => navigator.serviceWorker.ready);
  await page.waitForFunction(() => navigator.serviceWorker.controller);
  await page.waitForTimeout(1200);
}
async function opened(page, v) {
  await page.addInitScript(() => localStorage.setItem('lull:installOffered', 'true'));
  await hubIs(page, v);
  await openScreen(page, '/home');
  await started(page);
}

test('one Reload finishes an update, and the note does not come back', async ({ page }) => {
  await opened(page, V1);
  await expect(note(page)).toHaveCount(0);
  expect(await running(page)).toBe(V1);

  // the hub is replaced by a newer build while the page is open
  await hubIs(page, V2);
  await page.evaluate(() => App.checkHealth());
  await expect(note(page)).toHaveCount(1);
  await page.evaluate(() => App.checkHealth());
  await expect(note(page), 'asked again, the same change is not offered twice').toHaveCount(1);

  await note(page).locator('[data-toast-action]').click();
  await page.waitForEvent('load');
  await started(page);
  await expect(note(page), 'the page runs the new version: nothing to offer').toHaveCount(0);
  expect(await page.evaluate(() => App.server.version)).toBe(V2);
  expect(await running(page), 'the worker is the new version\'s').toBe(V2);

  // and no later check, reconnect or reload finds anything to offer
  await page.evaluate(() => App.checkHealth());
  await page.reload();
  await started(page);
  await expect(note(page)).toHaveCount(0);
});

test('closing the note keeps it away through reloads, until the hub changes again', async ({ page }) => {
  await opened(page, V1);
  await hubIs(page, V2);
  await page.evaluate(() => App.checkHealth());
  await note(page).locator('[data-toast-close]').click();
  await expect(note(page)).toHaveCount(0);
  for (let i = 0; i < 2; i++) {
    await page.reload();
    await started(page);
    await expect(note(page), `after reload ${i + 1}`).toHaveCount(0);
  }
  await hubIs(page, '1.0.0+3.cccccccc');
  await page.evaluate(() => App.checkHealth());
  await expect(note(page), 'a newer build is news again').toHaveCount(1);
});

test('an answer from the hub with no version offers nothing', async ({ page }) => {
  await opened(page, V1);
  await hubIs(page, null);
  await page.evaluate(() => App.checkHealth());
  await page.waitForTimeout(300);
  await expect(note(page)).toHaveCount(0);
  expect(await page.evaluate(() => App.server.version), 'the version it knew is kept').toBe(V1);
  await hubIs(page, V1);
  await page.evaluate(() => App.checkHealth());
  await page.waitForTimeout(300);
  await expect(note(page), 'and the same version again is no change').toHaveCount(0);
});

test('a page started on a newer hub than its worker takes the worker over without a note', async ({ page }) => {
  await opened(page, V1);
  await hubIs(page, V2);
  await page.reload();                       // not through the note: a relaunched app, a pull to refresh
  await started(page);
  await expect(note(page)).toHaveCount(0);
  await expect.poll(() => running(page)).toBe(V2);
});

// The note stays away only once the page really runs the new version. A page that started without its hub runs the
// files its worker kept (the old version); when the hub answers again as a newer build, that is news, and one Reload
// switches it.
test('a page started without its hub is offered the newer hub when it answers, and one Reload switches it', async ({ page }) => {
  await opened(page, V1);
  // the page starts while the hub can't be reached (its /health fails), so it runs what the V1 worker kept
  await page.unroute('**/health');
  await page.route('**/health', (route) => route.abort('internetdisconnected'));
  await page.reload();
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await page.waitForTimeout(500);
  await hubIs(page, V2);
  await page.evaluate(() => App.checkHealth());
  await expect(note(page), 'still running the kept old version: the newer hub is news').toHaveCount(1);
  await page.evaluate(() => App.checkHealth());
  await expect(note(page)).toHaveCount(1);

  await note(page).locator('[data-toast-action]').click();
  await page.waitForEvent('load');
  await started(page);
  await expect(note(page)).toHaveCount(0);
  await expect.poll(() => running(page)).toBe(V2);
  await page.evaluate(() => App.checkHealth());
  await page.waitForTimeout(300);
  await expect(note(page)).toHaveCount(0);
});
