// "Save songs you like" (library.autoSaveLikes, ADR-0099): the hub saves a song liked on its own; songs liked together
// are a batch it saves none of, so the web asks once. Turning the setting on asks once about the songs liked before.

import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

/** This test's own scope on the fixture hub (X-Fixture-Client): its settings and likes log are its own. */
async function own(page, request) {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  return {
    autoSave: (on) => request.patch('/api/settings', { data: { library: { autoSaveLikes: on } }, headers }),
    log: () => request.get('/__fixture/likes-log', { headers }).then((r) => r.json()),
    like: (id) => request.put('/api/likes/' + id, { headers }),
  };
}

/** Songs neither liked nor saved, made known to the page as search would. */
const fresh = (page, n) => page.evaluate((n) => App.data.api('/__fixture/unsaved-songs?n=' + n).then((r) => { App.data.absorb(r.entities); return r.items; }), n);

test('turning it on asks once about the songs liked before, and saves them on the word', async ({ page, request }) => {
  const hub = await own(page, request);
  await openScreen(page, '/settings/library');
  const [earlier] = await fresh(page, 1);
  await hub.like(earlier);
  const row = page.locator('.switch-row', { hasText: 'Save songs you like' });
  await expect(row.getByRole('switch')).toHaveAttribute('aria-checked', 'false');
  await row.getByRole('switch').click();
  const ask = page.getByRole('dialog');
  await expect(ask).toContainText('also save the songs you liked before?');
  await ask.getByRole('button', { name: /^Save \d/ }).click();
  await expect(page.locator('.toast', { hasText: /^Saving \d/ })).toBeVisible();
  expect((await hub.log()).saved.length).toBeGreaterThan(0);
});

test('a song liked on its own is liked alone; many liked at once are a batch the web asks about', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.autoSave(true);
  await openScreen(page, '/home');
  const [one, ...many] = await fresh(page, 3);
  expect(many.length).toBe(2);

  await page.evaluate((id) => App.toggleLike(id, true), one);
  await expect(page.locator('.toast', { hasText: 'Added to Liked songs' })).toBeVisible();
  await expect(page.locator('.toast', { hasText: /too\?/ })).toHaveCount(0);      // one song: the hub saves it, nothing to ask
  await page.evaluate((list) => App.setLiked(list, true), many);
  await expect(page.locator('.toast', { hasText: 'Save these 2 too?' })).toBeVisible();

  const { likes } = await hub.log();
  expect(likes.find((l) => l.trackId === one).batch).toBe(false);
  expect(likes.filter((l) => many.includes(l.trackId)).every((l) => l.batch)).toBe(true);
});

test('off, nothing asks and nothing is said about saving', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.autoSave(false);
  await openScreen(page, '/home');
  const list = await fresh(page, 2);
  await page.evaluate((l) => App.setLiked(l, true), list);
  await page.waitForTimeout(500);
  await expect(page.locator('.toast', { hasText: /too\?/ })).toHaveCount(0);
});
