// Now Playing says the same in every design, so pages from different designs can be mixed: with a song playing and
// its lyrics asked for in the side panel, Now Playing (at a phone, a laptop, a short and a wide window) and the Now
// column show its title, artist, album, quality and where it plays from, and its lyrics in one place only: the page
// when it is Now Playing, else the Now column, else the panel (docs/context/web.md, Layout, "One place shows the lyrics").
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { chromeNow } from '../checks.mjs';
import { designs, ids, openScreen } from './lib.mjs';

const sizes = {
  wide: { viewport: { width: 1920, height: 1080 } },
  laptop: { viewport: { width: 1366, height: 768 } },
  short: { viewport: { width: 1440, height: 700 } },
  phone: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
};

/* what the surface shows, read in the page: each fact is visible text somewhere inside it */
function facts(where) {
  const shown = (el) => el.getClientRects().length > 0 && getComputedStyle(el).visibility !== 'hidden' && !el.closest('[hidden], .vh');
  const root = document.querySelector(where);
  const t = App.player.current();
  const says = (text) => [...root.querySelectorAll('*')].some((e) => shown(e) && [...e.childNodes].some((n) => n.nodeType === 3 && n.textContent.includes(text)));
  return {
    title: says(t.title), artist: says(t.artist), album: says(t.album),
    quality: [...root.querySelectorAll('.q-chip')].some(shown),
    'playing from': [...root.querySelectorAll('[class*="__from"]')].some((e) => shown(e) && /playing from/i.test(e.textContent)),
    lyrics: [...document.querySelectorAll('.now-ly')].filter(shown).map((e) => /now-ly--(\w+)/.exec(e.className)[1]),
  };
}

async function playWithLyricsAsked(page, phone) {
  if (phone) await page.click('.tlist .trow:nth-child(1) .trow__title');
  else await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
  await page.waitForFunction(() => App.player.current() && App.player.state.playing);
  await page.evaluate(() => { App.player.pause(); if (window.innerWidth >= 1360) App.openPanel('lyrics'); });
}

for (const [size, use] of Object.entries(sizes)) {
  test.describe(size, () => {
    test.use(use);
    for (const design of designs) {
      test(`now playing / ${design}`, async ({ page }) => {
        await openScreen(page, `/album/${ids.album}`, { design, clock: false });
        await playWithLyricsAsked(page, size === 'phone');
        await page.evaluate(() => { location.hash = '#/now'; });
        await page.waitForFunction(() => document.querySelector('.now-ly--page .now-ly__line, .now-ly--page [data-state="none"]'));
        const f = await page.evaluate(facts, '.main');
        const missing = Object.entries(f).filter(([, v]) => v === false).map(([k]) => k);
        expect(missing, 'facts Now Playing leaves out').toEqual([]);
        expect(f.lyrics, 'where the lyrics show').toEqual(size === 'phone' ? [] : ['page']);
      });
    }
  });
}

/* what doesn't work on Now Playing, judged by what it does and never by its shape (ADR-0083; checks.mjs chromeNow): a
   control nothing can click, text something else covers, text on other text, text cut without an ellipsis; and a
   clamped line drawn past its clamp. Its panels may stand over each other, and its title over its art, as a design
   draws them; what it scrolls out of sight is not off the screen. Judged once the page has arrived: a phone slides a
   page in, and mid-way it is still below the screen (every animation that ends has ended; loops never do) */
async function overlaps(page) {
  await page.evaluate(() => Promise.race([
    Promise.all(document.getAnimations().filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime)).map((a) => a.finished.catch(() => {}))),
    new Promise((r) => setTimeout(r, 3000)),
  ]));
  const found = await page.evaluate(chromeNow, [['now', '#view', { scrolls: true }]]);
  return [...found, ...await page.evaluate(() => [...document.querySelectorAll('#view *')].filter((e) => {
    const s = getComputedStyle(e);
    return s.webkitLineClamp !== 'none' && s.overflowY === 'visible' && e.getClientRects().length && s.visibility !== 'hidden' && e.scrollHeight > e.clientHeight + 2;
  }).map((e) => '.' + [...e.classList].join('.') + ' draws past its clamp'))];
}
const LONG_TITLE = "Part III-Sampson & Shane-Fisher's Legend-The Big Idea";

for (const [size, use] of Object.entries(sizes)) {
  test.describe(size, () => {
    test.use(use);
    for (const design of designs) {
      test(`a long title overlaps nothing / ${design}`, async ({ page }) => {
        await openScreen(page, `/album/${ids.album}`, { design, clock: false });
        await playWithLyricsAsked(page, size === 'phone');
        await page.evaluate((t) => { App.player.current().title = t; location.hash = '#/now'; }, LONG_TITLE);
        await page.waitForFunction(() => document.querySelector('.now-ly--page .now-ly__line, .now-ly--page [data-state="none"], .now-head__title'));
        await page.evaluate(() => document.fonts.ready);
        expect(await overlaps(page), 'overlaps with the source details shut').toEqual([]);
        const open = page.locator('#view .now-srccard .now-src__toggle:visible');
        if (await open.count()) {
          await open.first().click();
          await page.waitForFunction(() => document.querySelector('#view .now-src__details:not([hidden])'));
          expect(await overlaps(page), 'overlaps with the source details open').toEqual([]);
        }
      });
    }
  });
}

test.describe('now column', () => {
  test.use(sizes.wide);
  for (const design of designs) {
    test(`now column / ${design}`, async ({ page }) => {
      await openScreen(page, `/album/${ids.album}`, { design, clock: false });
      await playWithLyricsAsked(page, false);
      await page.evaluate(() => App.layout.set({ nowColumn: 'open' }));
      await page.waitForFunction(() => document.querySelector('.now-ly--col .now-ly__line'));
      const f = await page.evaluate(facts, '.nowcol');
      // the quality is in its source card, which the fixture's songs leave loading while paused; Now Playing has it
      const missing = Object.entries(f).filter(([k, v]) => v === false && k !== 'quality').map(([k]) => k);
      expect(missing, 'facts the Now column leaves out').toEqual([]);
      expect(f.lyrics, 'where the lyrics show').toEqual(['col']);
    });
  }
});

// Why a song stopped is its job's, the hub's one record (docs/context/streaming.md, A song's outcome): the bar shows the
// hub's reason, a copy this browser can't decode as the hub recorded it, and only an unreachable hub in the player's words.
const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;
const playAlbum = async (page, album) => {
  // its own client on the fixture hub: a test playing the same song at the same time doesn't reopen this one's
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  await openScreen(page, `/album/${album}`, { clock: false });
  // its first song alone: every song of the failing album fails, so a whole album skips on through them, and between two
  // the player shows "Finding a source" (whole, with no tip to show); one song stays stopped with its reason
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')).slice(0, 1), 0));
  await page.waitForFunction(() => document.querySelector('#pbar').dataset.status === 'failed', null, { timeout: 15_000 });
  return page.evaluate(() => document.querySelector('#pbar').dataset.statusText);
};

test('a song the hub failed shows the hub\'s own reason, YouTube\'s part included', async ({ page }) => {
  const { failingAlbum } = await import('../fixture-hub/library.mjs');
  const text = await playAlbum(page, failingAlbum);
  expect(text).toBe('no peer shares this song right now; YouTube: no YouTube upload matches this song');
});

// The job's opening event comes on the event stream, its outcome on the play's own request: the event can arrive last.
test('a song that stopped stays stopped when its job\'s opening event comes late', async ({ page, request }) => {
  const { failingAlbum } = await import('../fixture-hub/library.mjs');
  await playAlbum(page, failingAlbum);
  const id = await page.evaluate(() => {
    const bar = document.querySelector('#pbar'); window.seenStatus = [];
    new MutationObserver(() => window.seenStatus.push(bar.dataset.status)).observe(bar, { attributes: true, attributeFilter: ['data-status'] });
    return App.player.current().id;
  });
  const emit = (data) => request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() },
    data: { topic: 'stream', data: { trackId: id, jobId: 'fixture-fail-' + id, attempt: 0, ...data } } });
  await emit({ status: 'finding', peersFound: 0, received: 0, size: 0 });
  await emit({ status: 'failed', error: 'the same job, failed again' });   // one stream, in order: the late event came first
  await expect.poll(() => page.evaluate(() => document.querySelector('#pbar').dataset.statusText)).toBe('the same job, failed again');
  expect(await page.evaluate(() => window.seenStatus)).not.toContain('finding');
});

test('a copy this browser can\'t decode shows what the hub recorded for its job', async ({ page }) => {
  const { brokenAlbum } = await import('../fixture-hub/library.mjs');
  const text = await playAlbum(page, brokenAlbum);
  expect(text).toBe('this copy can\'t be played in this player (WAV 8 kHz)');
});

test('with the hub out of reach, the bar says only that', async ({ page }) => {
  const { failingAlbum } = await import('../fixture-hub/library.mjs');
  await page.route('**/api/stream/*/output-error', (route) => route.abort('internetdisconnected'));
  const text = await playAlbum(page, failingAlbum);
  expect(text).toBe('Can\'t reach the lull server. Check your connection.');
});

// The reason is cut off where the bar is narrow (the state tab on a laptop, the mini player on a phone): its tooltip has it
// whole, on hover, on keyboard focus and on a tap, in a design that hides tooltips on phones too.
const tipText = (page) => page.evaluate(() => { const t = document.getElementById('tooltip');
  return t && t.classList.contains('is-on') && t.getBoundingClientRect().width > 0 ? t.textContent : null; });

test.describe('the reason a song stopped, cut off in the bar', () => {
  test.use(sizes.laptop);
  test('is whole in its tooltip on hover and on keyboard focus', async ({ page }) => {
    const { failingAlbum } = await import('../fixture-hub/library.mjs');
    const text = await playAlbum(page, failingAlbum);
    const line = page.locator('#pbar-state .pbar__state-line');
    expect(await line.evaluate((el) => el.scrollWidth > el.clientWidth), 'the fixture\'s reason is cut off in the tab').toBe(true);
    await line.hover(); await expect.poll(() => tipText(page)).toBe(text);
    await page.mouse.move(5, 5); await page.mouse.down(); await page.mouse.up();
    await page.keyboard.press('Shift'); await line.focus(); await expect.poll(() => tipText(page)).toBe(text);
  });
});

test.describe('the reason a song stopped, cut off in the mini player', () => {
  test.use(sizes.phone);
  test('is whole on a tap, and the title still opens Now Playing', async ({ page }) => {
    const { failingAlbum } = await import('../fixture-hub/library.mjs');
    const text = await playAlbum(page, failingAlbum);
    await page.tap('#mini-sub'); await expect.poll(() => tipText(page)).toBe(text);
    // a tap's compatibility mouseover comes between its pointerdown and its click, or doesn't, as the page's timing has it;
    // when it does, its hover timer must not show the tip again unasked (hidden on a phone) a moment later. And after the
    // tap the pointer rests where the finger was, so the browser sends mouseovers as the page changes under it (seen in
    // CI: one to the app's background as the tip appears); they must not close the tip the tap asked for. Send both
    await page.evaluate(() => { const el = document.getElementById('mini-sub');
      for (const e of [new PointerEvent('pointerdown', { bubbles: true, pointerType: 'touch' }), new MouseEvent('mouseover', { bubbles: true }), new MouseEvent('click', { bubbles: true, cancelable: true })]) el.dispatchEvent(e);
      document.getElementById('app').dispatchEvent(new MouseEvent('mouseover', { bubbles: true })); });
    await page.waitForTimeout(700); expect(await tipText(page), 'the tap\'s tip, still shown after the hover delay').toBe(text);
    expect(await page.evaluate(() => location.hash)).not.toContain('/now');
    await page.tap('#mini-title'); await expect.poll(() => page.evaluate(() => location.hash)).toContain('/now');
  });
});
