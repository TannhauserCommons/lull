// The phone's chrome, at 390 px (an iPhone) in every design: Now Playing's play button is a whole, obvious key (never a
// sliver in a part that gave way its height), the Search tab opens the keyboard on the search field, and nothing makes the
// page wider than the screen or zooms it (an input under 16 px zooms iOS Safari in and leaves it there).
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, ids, openScreen } from './lib.mjs';

test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

async function playSomething(page) {
  await page.click('.tlist .trow:nth-child(1) .trow__title');
  await page.waitForFunction(() => App.player.current());
}

for (const design of designs) {
  test(`the play button is a whole key on Now Playing / ${design}`, async ({ page }) => {
    await openScreen(page, `/album/${ids.album}`, { design, clock: false });
    await playSomething(page);
    await page.evaluate(() => { location.hash = '#/now'; });
    await page.waitForFunction(() => document.querySelector('#view [data-act="play"]'));
    await page.waitForTimeout(1200);                       // the page slides in
    const r = await page.evaluate(() => {
      const b = document.querySelector('#view [data-act="play"]');
      b.scrollIntoView({ block: 'center' });
      const q = b.getBoundingClientRect();
      const hit = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--hit'));
      // what fingers on a grid over the key reach: the key itself, not the part that clips it or something over it
      let reached = 0;
      for (const fx of [.15, .35, .5, .65, .85]) for (const fy of [.15, .35, .5, .65, .85]) {
        const at = document.elementFromPoint(q.x + q.width * fx, q.y + q.height * fy);
        if (at && b.contains(at)) reached++;
      }
      return { w: q.width, h: q.height, hit, reached, x: q.x, right: q.right };
    });
    expect(r.hit, '--hit is the touch target').toBeGreaterThan(0);
    expect(r.w, 'width').toBeGreaterThanOrEqual(r.hit);
    expect(r.h, 'height').toBeGreaterThanOrEqual(r.hit);
    expect(r.reached, 'of 25 points over the key that reach it (a round key has corners)').toBeGreaterThanOrEqual(17);
    expect(r.x, 'inside the screen').toBeGreaterThanOrEqual(0);
    expect(r.right, 'inside the screen').toBeLessThanOrEqual(390);
  });
}

// Every screen a listener can type on, and the others the key screens cover. A field under 16 px zooms iOS Safari in when
// it takes focus and leaves it zoomed; so does a page wider than the screen (it starts zoomed out).
const ROUTES = ['/home', '/search', '/search?q=velvet', '/library', '/library/songs', '/playlists', `/playlist/${ids.playlist}`, `/album/${ids.album}`,
  `/artist/${ids.artist}`, '/now', '/stats', '/settings', '/settings/playback', '/settings/sources', '/liked', '/history', '/downloads'];

/* the fields that take typing and the dropdowns, as drawn now (a checkbox, a range and a button never zoom the page) */
function smallFields() {
  const typed = /^(text|search|email|url|tel|password|number|date|datetime-local|month|time|week)?$/;
  return [...document.querySelectorAll('input, textarea, select, [contenteditable=""], [contenteditable="true"]')].filter((e) => {
    if (e.matches('input') && !typed.test(e.getAttribute('type') || '')) return false;
    return e.getClientRects().length > 0 && getComputedStyle(e).visibility !== 'hidden' && !e.closest('[hidden], .vh');
  }).map((e) => ({ el: e.tagName.toLowerCase() + (e.id ? '#' + e.id : '') + (e.className ? '.' + String(e.className).trim().split(/\s+/).join('.') : ''), px: parseFloat(getComputedStyle(e).fontSize) }))
    .filter((f) => f.px < 16);
}

for (const [size, use] of Object.entries({ phone: {}, 'tablet (the header search shows)': { viewport: { width: 820, height: 1180 } } })) {
  test.describe(size, () => {
    test.use(use);
    for (const design of designs) {
      for (const theme of ['day', 'night']) {
        test(`no field under 16 px and no sideways scroll / ${design} / ${theme}`, async ({ page }) => {
          const found = [];
          await openScreen(page, '/home', { design, theme, clock: false });
          for (const route of ROUTES) {
            await page.evaluate((r) => { location.hash = '#' + r; }, route);
            await page.waitForTimeout(500);
            await page.evaluate(() => document.fonts.ready);
            for (const f of await page.evaluate(smallFields)) found.push(`${route}: ${f.el} is ${f.px}px`);
            const wide = await page.evaluate(() => Math.max(document.documentElement.scrollWidth, document.body.scrollWidth) - innerWidth);
            if (wide > 0) found.push(`${route}: the page is ${wide}px wider than the screen`);
          }
          expect(found, 'fields iOS zooms into, and pages wider than the screen').toEqual([]);
        });
      }
    }
  });
}

test('the Search tab puts the cursor in the search field, from another page and on search itself', async ({ page }) => {
  await openScreen(page, '/home');
  const field = page.locator('#disc-q');
  const tab = page.locator('[data-region="tabbar"] [data-nav="search"]');
  await tab.tap();
  await expect(field).toBeFocused();
  await expect(page.locator('input[aria-hidden="true"][type="search"]'), 'the stand-in for the keyboard is gone').toHaveCount(0);
  await page.keyboard.type('amber');
  await expect(field).toHaveValue('amber');
  await field.blur();
  await tab.tap();
  await expect(field, 'pressed again on search').toBeFocused();
  await expect.poll(() => page.evaluate(() => document.querySelector('#disc-q').selectionStart), 'the cursor after the words').toBe(5);
});
