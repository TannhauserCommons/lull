// Key screens in every design and theme at a phone, a laptop and a wide window, on the fixture hub: each must load
// without errors or failed requests, keep inside the window, give every title room, line up
// the tags side by side, clip what they clamp, show each cover whole in the rows that scroll it, and match its golden.
// Approving an intended change: docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { clippedCovers, overflow, settle, squeezed, unclippedClamps, unevenTags, watch } from '../checks.mjs';
import { designs, NOW, screens, themes } from './lib.mjs';

const sizes = {
  phone: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
  laptop: { viewport: { width: 1366, height: 768 } },
  wide: { viewport: { width: 1920, height: 1080 } },
};

for (const [size, use] of Object.entries(sizes)) {
  test.describe(size, () => {
    test.use(use);
    for (const [name, route] of Object.entries(screens)) {
      for (const design of designs) {
        for (const theme of themes) {
          test(`${name} / ${design} / ${theme}`, async ({ page }) => {
            const problems = watch(page);
            await page.clock.setFixedTime(NOW);
            await page.addInitScript(([d, t]) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: t })), [design, theme]);
            await page.goto(`/#${route}`);
            await settle(page, problems);

            expect.soft(await overflow(page), 'content wider than the viewport').toBeNull();
            expect.soft(await squeezed(page), 'a title squeezed to a few letters').toBeNull();
            expect.soft(await unevenTags(page), 'tags side by side of different heights or off one centre line').toBeNull();
            expect.soft(await unclippedClamps(page), 'text clamped to some lines that draws the lines past them').toBeNull();
            expect.soft(await clippedCovers(page), 'a cover cut by the row that scrolls it').toBeNull();
            expect.soft(problems.console, 'console errors').toEqual([]);
            expect.soft(problems.requests, 'failed requests').toEqual([]);
            await expect(page).toHaveScreenshot(`${size}/${name}-${design}-${theme}.png`);
          });
        }
      }
    }
  });
}
