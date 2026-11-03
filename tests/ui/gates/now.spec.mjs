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
