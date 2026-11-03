// Classic mode's code (webamp-session.js, skins.js, classic.js) loads at start only in the desktop app; a browser loads
// it on first use (app.js App.needClassic) and never loads desktop.js. These pin what that must not change: a .wsz
// dropped anywhere becomes the Classic skin, leaving Classic returns to the page it was opened from, and the desktop
// app hears Webamp's window from the start (its crash recovery hands the queue back through classic.js).

import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

const loaded = (page) =>
  page.evaluate(() => ({
    classic: !!window.LullWebamp && !!App.classicSettings,
    desktop: !!window.LullDesktop,
    scripts: [...document.scripts].map((s) => s.getAttribute('src')).filter((s) => /classic|skins|webamp-session|desktop\.js/.test(s || '')),
  }));

test.describe('in a browser', () => {
  test('starts without Classic mode or the desktop bridge', async ({ page }) => {
    await openScreen(page, '/home');
    expect(await loaded(page)).toEqual({ classic: false, desktop: false, scripts: [] });
  });

  test('a .wsz dropped anywhere becomes the Classic skin', async ({ page }) => {
    await openScreen(page, '/home');
    await page.evaluate(() => {
      const dt = new DataTransfer();
      dt.items.add(new File(['PK\x03\x04 a skin'], 'Night Owl.wsz', { type: 'application/zip' }));
      document.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
    });
    await page.waitForFunction(() => App.classicSkin && App.classicSkin.get() && App.classicSkin.get().name === 'Night Owl');
  });

  test('leaving Classic returns to the page it was opened from', async ({ page }) => {
    await openScreen(page, `/album/${ids.album}`);
    await page.keyboard.press('w');
    await page.locator('[data-act="cls-exit"]').first().click();
    await page.waitForFunction((album) => location.hash === `#/album/${album}`, ids.album);
  });

  test('Settings > Appearance shows Classic mode\'s group', async ({ page }) => {
    await openScreen(page, '/settings/appearance');
    await expect(page.locator('#cls-browser, [data-skin-file]').first()).toBeAttached();
  });
});

test.describe('in the desktop app', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
  });

  test('loads Classic mode and the bridge with the page, and hears Webamp\'s window from the start', async ({ page }) => {
    await openScreen(page, '/home');
    const l = await loaded(page);
    expect(l.classic && l.desktop).toBe(true);
    // what the window reports after a crash reaches classic.js without Classic having been opened on this page
    await page.evaluate(() => window.__bridge.fire('classic', { type: 'failed', message: 'pinned by the gate' }));
    await expect(page.locator('.toast', { hasText: 'pinned by the gate' })).toBeVisible();
  });
});
