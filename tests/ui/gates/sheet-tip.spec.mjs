// A tooltip never shows away from what it names. A sheet slides up from the bottom of a phone, and its Close button takes
// focus while it is still on the way: the tip was placed there, at the screen's bottom edge, and stayed when the sheet
// arrived. On a phone nothing is hovered, and a control the app focused, not a key, asks for no tip.
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, openScreen } from './lib.mjs';

test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

/* every tooltip drawn, with how far its box is from the control it is for (the nearest edge, px; a tip sits 8 px off) */
const shownTips = () => [...document.querySelectorAll('.tooltip')]
  .filter((t) => getComputedStyle(t).display !== 'none' && +getComputedStyle(t).opacity > 0.01)
  .map((t) => {
    const a = t.getBoundingClientRect(), b = document.activeElement.getBoundingClientRect();
    return { text: t.textContent, gap: Math.max(b.left - a.right, a.left - b.right, b.top - a.bottom, a.top - b.bottom, 0) };
  });

for (const design of designs) {
  test(`a sheet's Close button shows no tooltip away from it / ${design}`, async ({ page }) => {
    await openScreen(page, '/home', { design });
    await page.evaluate(() => App.openSheet({ title: 'Source details', body: '<p>Tidal River</p>', foot: '<button class="btn btn--primary">Save</button>' }));
    await page.waitForFunction(() => document.activeElement && document.activeElement.matches('[data-close]'));
    for (const wait of [0, 100, 400, 800]) {                // while it slides up, and once it has arrived
      await page.waitForTimeout(wait);
      for (const tip of await page.evaluate(shownTips)) expect(tip.gap, `"${tip.text}" is ${tip.gap}px from Close at ${wait} ms`).toBeLessThanOrEqual(12);
    }
    const sheet = await page.locator('.sheet').boundingBox();
    expect(sheet.y + sheet.height, 'the sheet has arrived at the bottom').toBeGreaterThan(843);
  });

  // a key can reach Close while the sheet is still on the way: that tip is asked for, and follows the button
  test(`a tip asked for by a key follows Close up the sheet / ${design}`, async ({ page }) => {
    await openScreen(page, '/home', { design });
    await page.evaluate(() => { App.openSheet({ title: 'Source details', body: '<p>Tidal River</p>', autofocus: false }); document.querySelector('.sheet [data-close]').focus(); });
    await page.keyboard.press('Shift+Tab');
    await page.keyboard.press('Tab');
    await page.waitForFunction(() => document.activeElement && document.activeElement.matches('.sheet [data-close]'));
    for (const wait of [0, 100, 400, 800]) {
      await page.waitForTimeout(wait);
      for (const tip of await page.evaluate(shownTips)) expect(tip.gap, `"${tip.text}" is ${tip.gap}px from Close at ${wait} ms`).toBeLessThanOrEqual(12);
    }
    expect((await page.evaluate(shownTips)).length, 'the tip is up once the sheet has arrived, in a design that shows tips on a phone').toBe(design === 'lull' ? 0 : 1);
  });
}
