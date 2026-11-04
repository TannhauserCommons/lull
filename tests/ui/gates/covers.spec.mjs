// How covers are fetched (docs/context/web.md "Covers at the size they are drawn"): each picture asked for once per
// page, at the size its box needs in device pixels, and a cover the hub is still fetching asked again only when
// `art.ready` names it, never on a timer. On the fixture hub, so every count is exact.

import { expect, test } from '@playwright/test';
import { ids, openScreen, ownHub, throttle } from './lib.mjs';

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
      // slowed, as a low-end device (and a slow runner) is: a render that lands while the first answer is on its way
      // must not ask again (a picture parsed into the live document, not the template's, fetched each render)
      await throttle(page, 6);
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

// The hub fetches outside pictures in the order screens show them (ADR-0033, PUT /api/pictures/screen). Live, a long
// list scrolled fast filled its covers in the order they were asked, so the rows scrolled past filled before the screen
// the listener stopped on. On a cold hub that gets one cover every PACE ms in that order, the covers on the screen a
// fast scroll stops on are the next ones it gets, and they are all shown within that many turns.
test('a long list of uncached covers scrolled fast fills the screen it stops on first', async ({ browser }) => {
  const PACE = 100;
  const hub = await ownHub({ FIXTURE_ALBUMS: '480', FIXTURE_LONG_PLAYLIST: '5000' });
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' });
  try {
    await fetch(hub.url + '/__fixture/art-cold', { method: 'POST', body: JSON.stringify({ pace: PACE }) });
    const page = await context.newPage();
    await openScreen(page, `/playlist/${ids.longPlaylist}`);
    // to about two thirds down, a screen and a half a frame: every row passed is drawn, and its covers asked for
    await page.evaluate(async () => {
      let s = document.querySelector('#view li[data-fill]');
      while (s && !(s.scrollHeight > s.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
      s = s || document.scrollingElement;
      for (let y = 0; y < s.scrollHeight * 0.66; y += 1350) { s.scrollTop = y; await new Promise((ok) => requestAnimationFrame(ok)); }
    });
    // the rows where it stopped draw their contents in the next frames (ui.row)
    await page.waitForFunction(() => ![...document.querySelectorAll('#view li[data-fill]:empty')]
      .some((li) => { const r = li.getBoundingClientRect(); return r.bottom > 0 && r.top < innerHeight; }));
    const had = await (await fetch(hub.url + '/__fixture/art-filled')).json();
    // on screen: inside the box the list scrolls in (under the header is not)
    const onScreen = await page.evaluate(() => {
      const box = document.querySelector('#view').getBoundingClientRect();
      return [...new Set([...document.querySelectorAll('#view [data-art], #view [data-wait-art]')]
        .filter((el) => { const r = el.getBoundingClientRect(); return r.width > 0 && r.bottom > box.top && r.top < box.bottom; })
        .map((el) => el.getAttribute('data-art') || el.getAttribute('data-wait-art')))];
    });
    expect(onScreen.length, 'covers on the screen stopped on').toBeGreaterThan(1);
    // the report goes out within a quarter second of the scroll stopping; a turn or two may go to the screen before
    const cold = onScreen.filter((k) => !had.includes(k)), turns = cold.length + 4;
    await page.waitForFunction((keys) => keys.every((k) => [...document.querySelectorAll(`#view img.cover__img[data-art="${k}"]`)]
      .some((i) => i.complete && i.naturalWidth > 0)), onScreen, { timeout: turns * PACE + 3000 }).catch(() => {});
    const next = (await (await fetch(hub.url + '/__fixture/art-filled')).json()).slice(had.length, had.length + turns);
    expect(cold.filter((k) => !next.includes(k)), 'covers on screen the hub did not get next').toEqual([]);
    const unshown = await page.evaluate((keys) => keys.filter((k) => ![...document.querySelectorAll(`#view img.cover__img[data-art="${k}"]`)]
      .some((i) => i.complete && i.naturalWidth > 0)), onScreen);
    expect(unshown, `covers on screen not shown within ${turns} turns`).toEqual([]);
  } finally {
    await context.close();
    hub.close();
  }
});

// An artist drawn without a photo asks the hub for one through its screen (size 0), and the photo the hub vetted later
// comes in art.ready's `images`: every cover of the artist shows it, with no page read again. Live, a vetted photo
// showed only once the page was read again.
test('an artist drawn without a photo shows the one the hub vets later', async ({ page, request }) => {
  const hub = await own(page, request);
  await openScreen(page, `/artist/${ids.artist}`);
  const asked = async () => {
    const s = await (await hub.get('/__fixture/screen')).json();
    return !!s && [...s.visible, ...s.near].some(([pic, size]) => pic === `catalog/art/${ids.artist}` && size === 0);
  };
  await expect.poll(asked, { message: 'the screen asks for the artist\'s photo' }).toBe(true);
  const reads = [];
  page.on('request', (r) => { if (/\/api\/artists\//.test(r.url())) reads.push(r.url()); });
  await hub.post('/__fixture/emit', { data: { topic: 'art.ready', data: { keys: [ids.artist], images: { [ids.artist]: `api/catalog/art/${ids.artist}?v=1` } } } });
  await page.waitForFunction((id) => {
    const covers = [...document.querySelectorAll(`#view [data-want-photo="${id}"], #view img.cover__img[data-art="${id}"]`)];
    return covers.length > 0 && covers.every((c) => c.tagName === 'IMG' && c.complete && c.naturalWidth > 0 && c.parentNode.classList.contains('has-art'));
  }, ids.artist, { timeout: 5000 });
  expect(reads, 'the artist page read again').toEqual([]);
});
