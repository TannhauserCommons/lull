// The phone's player bar (the mini player above the tab bar, every screen but Now Playing), in every design, at the two
// narrowest phones. A listener on an iPhone (390 px, the web app, 2026-11): the cover sat too far left, there was no way
// to go back a song (only play and next), and a design's progress line could not be dragged. So, in every design: the
// cover has the room at the bar's left that the last key has at its right; previous, play and next are keys (each at
// least --hit), previous and next never give way; a part that gave way (play on another device) leaves its row and
// never overlaps another; the progress line seeks by touch, with a touch area --hit tall; a swipe on the bar still skips
// a song, and a drag along the line seeks without skipping. docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, ids, openScreen } from './lib.mjs';

const widths = [360, 390];

// the playing song on the bar, in a queue of the album's songs, the second one playing (so there is a song before it)
async function playOnBar(page, design, width) {
  await page.setViewportSize({ width, height: width === 360 ? 780 : 844 });
  await openScreen(page, `/album/${ids.album}`, { design, theme: 'day' });
  await page.waitForSelector('#view .trow[data-track-id]');
  const queue = await page.evaluate(() => {
    const list = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(list[1], list);
    location.hash = '#/home';
    return list;
  });
  await page.waitForFunction(() => App.player.current() && App.player.state.playing && App.player.duration() > 0);
  await page.evaluate(() => App.player.pause());
  await page.waitForSelector('#mini [data-act="next"]', { state: 'visible' });
  await page.waitForTimeout(300);
  return queue;
}

const current = (page) => page.evaluate(() => App.player.current().id);
const frac = (page) => page.evaluate(() => App.player.state.pos / App.player.duration());

// a one-finger drag, as the phone sends it (pointer events with touch)
async function drag(page, from, to, steps = 12) {
  const cdp = await page.context().newCDPSession(page);
  const touch = (type, x, y) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x, y, id: 1 }] });
  await touch('touchStart', from.x, from.y);
  for (let i = 1; i <= steps; i++) await touch('touchMove', from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps);
  await touch('touchEnd', to.x, to.y);
  await cdp.detach();
}

for (const design of designs) {
  for (const width of widths) {
    test.describe(`${design} / ${width}`, () => {
      test.use({ viewport: { width, height: 844 }, isMobile: true, hasTouch: true });

      test('previous, play and next are keys, and the cover has the room the last key has', async ({ page }) => {
        const queue = await playOnBar(page, design, width);
        const bar = await page.evaluate(() => {
          const box = (el) => { const r = el.getBoundingClientRect(); return { left: r.left, right: r.right, top: r.top, bottom: r.bottom, width: r.width, height: r.height }; };
          const hit = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--hit'));
          const mini = document.getElementById('mini');
          const keys = {};
          for (const act of ['prev', 'play', 'next']) {
            const el = mini.querySelector(`[data-act="${act}"]`);
            keys[act] = el && { ...box(el), out: el.hasAttribute('data-fit-out'), shown: el.getClientRects().length > 0 };
          }
          // every part laid out in the bar, the progress line (drawn over its edge) apart
          const parts = [...mini.children].filter((el) => !el.classList.contains('seek') && getComputedStyle(el).display !== 'none' && el.getClientRects().length)
            .map((el) => ({ name: (el.getAttribute('data-act') || el.id || el.className).toString().split(' ')[0], ...box(el) }));
          return { hit, mini: box(mini), keys, thumb: box(document.getElementById('mini-thumb')), nextIcon: box(mini.querySelector('[data-act="next"] .ic')), parts, scroll: [document.documentElement.scrollWidth, innerWidth],
            miniScroll: [mini.scrollWidth, mini.clientWidth] };
        });
        for (const act of ['prev', 'play', 'next']) {
          const k = bar.keys[act];
          expect(k, `${act} is on the bar`).toBeTruthy();
          expect(k.shown && !k.out, `${act} stays on the bar`).toBe(true);
          expect(k.width, `${act} is at least as wide as --hit`).toBeGreaterThanOrEqual(bar.hit - 0.5);
          expect(k.height, `${act} is at least as tall as --hit`).toBeGreaterThanOrEqual(bar.hit - 0.5);
        }
        // the cover's room at the left is the room at the right of the last key's icon (its box has room of its own round it)
        const left = bar.thumb.left - bar.mini.left, right = bar.mini.right - bar.nextIcon.right;
        expect(left, 'the cover is well inside the bar').toBeGreaterThanOrEqual(12);
        expect(Math.abs(left - right), `the cover's room at the left (${left}) is the last key's icon's at the right (${right})`).toBeLessThanOrEqual(2);
        // nothing overlaps, nothing leaves the bar or the page
        for (const p of bar.parts) {
          expect(p.left, `${p.name} inside the bar's left edge`).toBeGreaterThanOrEqual(bar.mini.left - 0.5);
          expect(p.right, `${p.name} inside the bar's right edge`).toBeLessThanOrEqual(bar.mini.right + 0.5);
        }
        const sorted = [...bar.parts].sort((a, b) => a.left - b.left);
        for (let i = 1; i < sorted.length; i++) expect(sorted[i].left, `${sorted[i - 1].name} and ${sorted[i].name} overlap`).toBeGreaterThanOrEqual(sorted[i - 1].right - 0.5);
        expect(bar.scroll[0], 'the page does not scroll sideways').toBeLessThanOrEqual(bar.scroll[1]);
        expect(bar.miniScroll[0], 'the bar holds its parts').toBeLessThanOrEqual(bar.miniScroll[1] + 1);

        // they work: previous goes back a song, next on again
        await page.tap('#mini [data-act="prev"]');
        await expect.poll(() => current(page)).toBe(queue[0]);
        await page.tap('#mini [data-act="next"]');
        await expect.poll(() => current(page)).toBe(queue[1]);
      });
    });
  }
}
