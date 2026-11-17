// The queue's rows answer the mouse and keyboard as the row menu does, wherever they are drawn (the queue panel, and
// Now Playing's "up next" preview): a double-click jumps to the song like Enter, Shift+Enter moves it to play next,
// removing a song or clearing the queue leaves keyboard focus in the panel, and Escape ends a drag without closing the panel.
// docs/context/web.md, Queue.

import { expect, test } from '@playwright/test';
import { ids, openScreen, settle } from './lib.mjs';

test.use({ viewport: { width: 1440, height: 900 } });

const queueIds = (page) => page.evaluate(() => App.player.state.queue.map((q) => q.trackId));
const indexOf = (page) => page.evaluate(() => App.player.state.index);
const focused = (page) =>
  page.evaluate(() => {
    const a = document.activeElement;
    return { tag: a.tagName, row: a.closest?.('.now-q__row')?.getAttribute('data-qi') ?? null, inPanel: !!a.closest?.('.ctx') };
  });

// Plays the playlist's first songs, paused on the first.
async function playSome(page, design = 'lull') {
  await openScreen(page, `/playlist/${ids.playlist}`, { design });
  await page.evaluate(() => {
    App.player.playList([...document.querySelectorAll('#view li[data-track-id]')].slice(0, 8).map((r) => r.getAttribute('data-track-id')), 0, { autoplay: false });
    App.player.pause();
  });
}
async function openQueue(page, design) {
  await playSome(page, design);
  await page.evaluate(() => App.openPanel('queue'));
  await page.waitForSelector('.ctx .now-q__row[data-qi="3"]');
  await settle(page);
}
const row = (page, qi) => page.locator(`.ctx .now-q__row[data-qi="${qi}"]`);

test('a double-click on a queue row jumps to its song and keeps the queue', async ({ page }) => {
  await openQueue(page);
  const before = await queueIds(page);
  await row(page, 3).locator('.trow__artist span').nth(1).dblclick();
  const after = await queueIds(page);
  expect(after, 'the queue keeps every song').toHaveLength(before.length);
  expect(new Set(after)).toEqual(new Set(before));
  expect(await page.evaluate(() => App.player.current().id), 'the double-clicked song plays').toBe(before[3]);
});
test.describe('phone', () => {
test.use({ viewport: { width: 390, height: 844 } });
test('a double-click on an up-next row of Now Playing jumps to its song and keeps the queue', async ({ page }) => {
  await playSome(page);
  await page.evaluate(() => { location.hash = '#/now'; });
  const preview = page.locator('#view ol[aria-label="Up next"] .now-q__row').first();
  await preview.waitFor();
  await settle(page);
  const before = await queueIds(page);
  await preview.locator('.trow__artist span').nth(1).dblclick();
  expect(await queueIds(page), 'the queue keeps every song').toHaveLength(before.length);
  expect(await page.evaluate(() => App.player.current().id), 'the double-clicked song plays').toBe(before[1]);
});
});

test('Shift+Enter on a queue row moves its song to play next, not a copy', async ({ page }) => {
  await openQueue(page);
  const before = await queueIds(page);
  await row(page, 4).focus();
  await page.keyboard.press('Shift+Enter');
  const after = await queueIds(page);
  expect(after, 'no song is added').toHaveLength(before.length);
  expect(after[(await indexOf(page)) + 1], 'the song plays next').toBe(before[4]);
});

test('removing a song from its menu leaves focus on the next row', async ({ page }) => {
  await openQueue(page);
  const before = await queueIds(page);
  await row(page, 2).locator('[data-act="now-q-menu"]').click();
  await page.getByRole('menuitem', { name: 'Remove from queue' }).click();
  expect(await queueIds(page)).toEqual(before.filter((_, i) => i !== 2));
  expect(await focused(page), 'the song that took its place').toMatchObject({ row: '2', inPanel: true });
});

test('Clear leaves focus in the queue panel', async ({ page }) => {
  await openQueue(page);
  await page.locator('.ctx [data-act="now-q-clear"]').click();
  expect(await queueIds(page)).toHaveLength(1);
  expect(await focused(page), 'not dropped to the page').toMatchObject({ inPanel: true });
});

// below the width that docks the panel it opens over the page, where Escape closes it (not in lull, whose overlay panel is 20 px wide there)
test.describe('overlay panel', () => {
test.use({ viewport: { width: 800, height: 900 } });
test('Escape during a queue drag ends the drag and leaves the panel open', async ({ page }) => {
  await openQueue(page, 'skydeck');
  const before = await queueIds(page);
  // the overlay slides in over the page: take the row's place once it is in view
  await expect(row(page, 3)).toBeInViewport({ ratio: 1 });
  const box = await row(page, 3).boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2 + 60, { steps: 6 });
  await expect(page.locator('.ctx .now-q__list.is-sorting')).toBeAttached();
  await page.keyboard.press('Escape');
  await page.mouse.up();
  await expect(page.locator('.ctx .now-q__list.is-sorting')).toHaveCount(0);
  await expect(page.locator('#app')).toHaveAttribute('data-ctx', 'open');
  expect(await queueIds(page), 'nothing moved').toEqual(before);
});
});
