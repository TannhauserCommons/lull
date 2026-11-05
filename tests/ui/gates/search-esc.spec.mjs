// Esc on search is a real back (docs/context/web.md, Search): the first Esc in a field with words clears them; the next
// returns to the page search was opened from, where it was, past every search the listener submitted; with nothing
// before search (a fresh load), Home. A menu, popover or dialog open on search closes first.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

const at = (page) => page.evaluate(() => location.hash);
const scrolled = (page) => page.evaluate(() => document.querySelector('#view').scrollTop);

test('Esc clears the words, then returns to the page before search, where it was', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.longPlaylist}`);
  await page.evaluate(() => { document.querySelector('#view').scrollTop = 1200; });
  await page.waitForTimeout(150);
  const row = page.locator('#view .trow[data-track-id]').nth(20);
  const rowId = await row.getAttribute('data-track-id');
  await row.focus();
  const top = await scrolled(page);
  expect(top, 'the playlist scrolled').toBeGreaterThan(600);

  await page.keyboard.press('/');
  await expect(page.locator('#header-search')).toBeFocused();
  await page.keyboard.type('night');
  await expect.poll(() => at(page)).toBe('#/search?q=night');
  await page.keyboard.press('Enter');                      // a submitted search, then another: two search entries
  await page.locator('#header-search').fill('nightfold');
  await page.keyboard.press('Enter');
  await expect.poll(() => at(page)).toBe('#/search?q=nightfold');

  await page.keyboard.press('Escape');
  await expect(page.locator('#header-search')).toHaveValue('');
  await expect(page.locator('#header-search'), 'the first Esc clears the words and keeps the field').toBeFocused();
  expect(await at(page)).toBe('#/search');

  await page.keyboard.press('Escape');
  await expect.poll(() => at(page)).toBe(`#/playlist/${ids.longPlaylist}`);
  await expect.poll(() => scrolled(page), 'the playlist where it was').toBeGreaterThan(top - 40);
  expect(await scrolled(page)).toBeLessThan(top + 40);
  await expect.poll(() => page.evaluate(() => document.activeElement && document.activeElement.getAttribute('data-track-id')), 'focus on the song it was on').toBe(rowId);
});

test.describe('on a phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });
  test('the search page\'s own field: the first Esc clears, the next goes back', async ({ page }) => {
    await openScreen(page, '/library');
    await page.evaluate(() => { location.hash = '#/search'; });
    const field = page.locator('#disc-q');
    await field.fill('amber');
    await expect.poll(() => at(page)).toBe('#/search?q=amber');
    await page.keyboard.press('Escape');
    await expect(field).toHaveValue('');
    expect(await at(page)).toBe('#/search');
    await page.keyboard.press('Escape');
    await expect.poll(() => at(page)).toBe('#/library');
    await expect.poll(() => page.evaluate(() => document.activeElement && document.activeElement.closest('#view') !== null), 'focus in the page').toBe(true);
  });
});

test('on a fresh load of search, Esc goes Home', async ({ page }) => {
  await openScreen(page, '/search');
  await page.locator('#view').focus();
  await page.keyboard.press('Escape');
  await expect.poll(() => at(page)).toBe('#/home');
});

test('a popover open on search closes first', async ({ page }) => {
  await openScreen(page, '/home');
  await page.evaluate(() => { location.hash = '#/search'; });
  await expect.poll(() => at(page)).toBe('#/search');
  await page.evaluate(() => App.popover(document.querySelector('#view h1, #view h2, #view button'), '<p>a popover</p>'));
  await expect(page.locator('.popover')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(page.locator('.popover')).toHaveCount(0);
  expect(await at(page)).toBe('#/search');
  await page.keyboard.press('Escape');
  await expect.poll(() => at(page)).toBe('#/home');
});
