// Now Playing says the same in every design, so pages from different designs can be mixed: with a song playing and
// its lyrics asked for in the side panel, Now Playing (at a phone, a laptop, a short and a wide window) and the Now
// column show its title, artist, album, quality and where it plays from, and its lyrics in one place only: the page
// when it is Now Playing, else the Now column, else the panel (docs/context/web.md, Layout, "One place shows the lyrics").
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
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

/* what overlaps on Now Playing: two of its panels (more than the edge two joined windows share), two lines of its
   heading, the title's words and the action buttons, or a clamped line drawn past its clamp */
function overlaps() {
  const box = (e) => {
    if (!e || !e.getClientRects().length) return null;
    const s = getComputedStyle(e); if (s.visibility === 'hidden' || s.clipPath !== 'none') return null;
    const r = e.getBoundingClientRect(); return r.width > 2 && r.height > 2 ? r : null;
  };
  const meet = (a, b) => Math.min(Math.min(a.right, b.right) - Math.max(a.left, b.left), Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
  const name = (e) => '.' + [...e.classList].join('.');
  const view = document.querySelector('#view'), out = [];
  const panels = ['.now__dev', '.now__devc', '.now-head', '.now__acts', '.now-srccard', '.now-lycard', '.now-next', '.now-qcard']
    .map((s) => view.querySelector(s)).filter((e) => box(e));
  for (let i = 0; i < panels.length; i++) for (let j = i + 1; j < panels.length; j++) {
    const a = box(panels[i]), b = box(panels[j]);
    const tabs = Math.abs(a.left - b.left) < 1 && Math.abs(a.top - b.top) < 1;   // one window's pages, shown one at a time
    if (!tabs && meet(a, b) > 2) out.push(`${name(panels[i])} on ${name(panels[j])}`);
  }
  const head = view.querySelector('.now-head'), kids = head ? [...head.children].filter((e) => box(e)) : [];
  for (let i = 0; i < kids.length; i++) for (let j = i + 1; j < kids.length; j++) if (meet(box(kids[i]), box(kids[j])) > 1) out.push(`${name(kids[i])} on ${name(kids[j])}`);
  const title = view.querySelector('.now-head__title'), acts = box(view.querySelector('.now__acts'));
  if (title && acts && box(title)) {
    const words = document.createRange(); words.selectNodeContents(title);
    if ([...words.getClientRects()].some((r) => r.width && meet(r, acts) > 1)) out.push('the title on the actions');
  }
  for (const e of view.querySelectorAll('*')) {
    const s = getComputedStyle(e);
    if (s.webkitLineClamp !== 'none' && s.overflowY === 'visible' && box(e) && e.scrollHeight > e.clientHeight + 2) out.push(`${name(e)} draws past its clamp`);
  }
  return out;
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
        expect(await page.evaluate(overlaps), 'overlaps with the source details shut').toEqual([]);
        const open = page.locator('#view .now-srccard .now-src__toggle:visible');
        if (await open.count()) {
          await open.first().click();
          await page.waitForFunction(() => document.querySelector('#view .now-src__details:not([hidden])'));
          expect(await page.evaluate(overlaps), 'overlaps with the source details open').toEqual([]);
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
const playAlbum = async (page, album) => {
  await openScreen(page, `/album/${album}`, { clock: false });
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await page.waitForFunction(() => document.querySelector('#pbar').dataset.status === 'failed', null, { timeout: 15_000 });
  return page.evaluate(() => document.querySelector('#pbar').dataset.statusText);
};

test('a song the hub failed shows the hub\'s own reason, YouTube\'s part included', async ({ page }) => {
  const { failingAlbum } = await import('../fixture-hub/library.mjs');
  const text = await playAlbum(page, failingAlbum);
  expect(text).toBe('no peer shares this song right now; YouTube: no YouTube upload matches this song');
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
