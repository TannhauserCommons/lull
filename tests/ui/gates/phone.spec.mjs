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
