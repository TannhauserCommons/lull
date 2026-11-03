// The chrome fits at any width: in every design and theme, with a song loaded, the window is narrowed from 2560 to
// 360 px and widened back, 20 px at a time, and the header, the player bar, the mini player and the tab bar are
// checked at each step (checks.mjs `chrome`): nothing outside its bar or the window, no control on another, no text
// cut off, every essential there. The header and the bar fit themselves (layout.js, data-fit), so no design needs a
// width band to patch them; a new one that would is caught here. The desktop app's mini player is checked at its
// own sizes. docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { chrome } from '../checks.mjs';
import { designs, ids, openScreen, themes } from './lib.mjs';

const widths = [];
for (let w = 2560; w >= 360; w -= 20) widths.push(w);
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));

for (const design of designs) {
  for (const theme of themes) {
    test(`header and player bar / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');       // a song in the bar: its widest state
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForTimeout(500);
      const found = new Map();
      for (const w of [...widths, ...widths.slice().reverse()]) {    // both ways: a row fits again as it widens
        await page.setViewportSize({ width: w, height: 900 });
        await frames(page);
        for (const p of await chrome(page)) found.set(p, [...(found.get(p) || []), w]);
      }
      const report = [...found].map(([p, ws]) => `${p} at ${[...new Set(ws)].sort((a, b) => a - b).join(', ')} px`);
      expect(report, 'the chrome at some width').toEqual([]);
    });

    test(`desktop mini player / ${design} / ${theme}`, async ({ page }) => {
      // the device window opens 340 px wide, the strip is a fixed 540 x 90 (app/src/desktop/mini.rs)
      for (const [view, width] of [['mini', 340], ['strip', 540]]) {
        await page.setViewportSize({ width, height: 700 });
        await page.goto(`/desktop/wrapper.html#view=${view}&design=${design}&theme=${theme}&opaque=1`);
        await page.waitForSelector('[data-dk-player]');
        await page.evaluate(() => document.fonts.ready);
        expect(await chrome(page), `the ${view} at ${width} px`).toEqual([]);
      }
    });
  }
}
