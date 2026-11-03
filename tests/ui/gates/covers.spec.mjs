// How covers are fetched (docs/context/web.md "Covers at the size they are drawn"): each picture asked for once per
// page, at the size its box needs in device pixels, and a cover the hub is still fetching asked again only when
// `art.ready` names it, never on a timer. On the fixture hub, so every count is exact.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

/** This test's own scope on the fixture hub (server.mjs, X-Fixture-Client): its page's cover requests, pending covers
 *  and art.ready events, apart from the gates running beside it on the same hub. */
async function own(page, request) {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);   // the context's: the service worker's fetches carry it too
  return { get: (url) => request.get(url, { headers }), post: (url, o) => request.post(url, { ...o, headers }) };
}

/** Cover requests that reached the fixture hub while `fn` ran (the browser's cache hits never do): url -> count. */
async function coverRequests(hub, fn) {
  await hub.get('/__fixture/art-requests');
  await fn();
  return new Map(Object.entries(await (await hub.get('/__fixture/art-requests')).json()));
}

const unloaded = () => [...document.querySelectorAll('img.cover__img')]
  .filter((i) => { const r = i.getBoundingClientRect(); return r.width > 0 && r.bottom > 0 && r.top < innerHeight && !(i.complete && i.naturalWidth > 0); })
  .map((i) => i.getAttribute('data-art') + ' ' + i.getAttribute('src'));

for (const [name, route] of [['home', '/home'], ['the 1,200-song playlist', `/playlist/${ids.longPlaylist}`], ['an album', `/album/${ids.album}`]]) {
  test(`${name} asks for each cover once, and shows them within 2 s`, async ({ page, request }) => {
    const hub = await own(page, request);
    const seen = await coverRequests(hub, async () => {
      await openScreen(page, route);
      // every cover on screen loaded within 2 s of the page settling (nothing waits on a timer)
      await page.waitForFunction(() => [...document.querySelectorAll('img.cover__img')]
        .filter((i) => { const r = i.getBoundingClientRect(); return r.width > 0 && r.bottom > 0 && r.top < innerHeight; })
        .every((i) => i.complete && i.naturalWidth > 0), null, { timeout: 2000 }).catch(() => {});
      expect(await page.evaluate(unloaded), 'covers on screen not loaded 2 s after the page settled').toEqual([]);
    });
    const repeated = [...seen].filter(([, n]) => n > 1).map(([url]) => url);
    expect(repeated, 'cover URLs asked for more than once').toEqual([]);
    const inPage = await page.evaluate(() => document.querySelectorAll('img.cover__img[src]').length);
    expect(seen.size, 'cover requests against cover images in the page').toBeLessThanOrEqual(inPage);
  });
}

test('a cover the hub is still fetching is asked once, then again when art.ready names it', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.post('/__fixture/art-pending', { data: { ids: [ids.album] } });
  try {
    const mine = (seen) => [...seen].filter(([url]) => url.includes(ids.album));
    const first = await coverRequests(hub, async () => {
      await openScreen(page, `/album/${ids.album}`);
      await page.waitForTimeout(6000);   // longer than the 5 s the page used to retry after
    });
    // the header and every row show the same picture: one request per size drawn, answered "not yet", no retry
    expect(mine(first).length, 'sizes asked').toBeGreaterThan(0);
    expect(mine(first).filter(([, n]) => n > 1), 'sizes asked again before art.ready').toEqual([]);
    const then = await coverRequests(hub, async () => {
      await hub.post('/__fixture/art-ready', { data: { ids: [ids.album] } });
      await page.waitForFunction((id) => [...document.querySelectorAll(`img.cover__img[data-art="${id}"]`)]
        .some((i) => i.complete && i.naturalWidth > 0), ids.album, { timeout: 5000 });
    });
    expect(mine(then).length, 'sizes asked again after art.ready').toBeGreaterThan(0);
    expect(mine(then).filter(([, n]) => n > 1), 'each asked once more').toEqual([]);
  } finally {
    await hub.post('/__fixture/art-ready', { data: { ids: [ids.album] } });
  }
});

for (const scale of [1, 2]) {
  test.describe(`at ${scale}x`, () => {
    test.use({ deviceScaleFactor: scale, viewport: { width: 1440, height: 900 } });

    test('every cover asks for the smallest size that covers its box', async ({ page }) => {
      await openScreen(page, `/album/${ids.album}`);
      const wrong = await page.evaluate(() => {
        const sizes = [96, 256, 512, 1024, 2048];
        return [...document.querySelectorAll('img.cover__img[src]')].flatMap((i) => {
          const box = Math.max(i.getBoundingClientRect().width, i.getBoundingClientRect().height) * devicePixelRatio;
          const asked = +(/[?&]size=(\d+)/.exec(i.getAttribute('src')) || [])[1];
          // 10% short still counts (web.md); the fixture's pictures stop at 512 px, which the page must learn
          const want = Math.min(sizes.find((s) => s >= box * 0.9) || 2048, 512);
          return box && asked !== want ? [`${Math.round(box)} device px asked ${asked}, want ${want}`] : [];
        });
      });
      expect(wrong).toEqual([]);
    });
  });
}
