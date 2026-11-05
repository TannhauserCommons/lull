// Confirmations (ADR-0040): the notes that only confirm what the listener did collapse into one that counts up, its
// Undo undoes all of them, Quiet keeps them off the screen but still says them to a screen reader, and an alert or an
// error always shows.
import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

const toasts = (page) => page.locator('#toasts .toast');
const firstSongs = (page, n) => page.evaluate((n) => App.data.api('/api/feed').then((f) => App.data.api('/api/albums/' + f.newReleases[0]))
  .then((a) => (a.album || a).trackIds.slice(0, n)), n);

test('confirmations of one kind collapse into one note that counts up, and its Undo undoes them all', async ({ page }) => {
  await openScreen(page, '/home');
  const ids = await firstSongs(page, 3);
  await page.evaluate((ids) => ids.forEach((id) => App.player.addToQueue([id])), ids);
  await expect(toasts(page)).toHaveCount(1);
  await expect(toasts(page)).toContainText('Added 3 songs to the queue');
  await page.evaluate((ids) => ids.forEach((id) => App.toggleLike(id, true)), ids);
  await expect(toasts(page)).toHaveCount(2);
  const likes = toasts(page).filter({ hasText: 'Liked 3 songs' });
  await expect(likes).toHaveCount(1);
  await likes.locator('[data-toast-action]').click();
  expect(await page.evaluate((ids) => ids.map((id) => App.isLiked(id)), ids)).toEqual([false, false, false]);
});

test('Quiet keeps confirmations off the screen, says them to a screen reader, and still shows alerts and errors', async ({ page }) => {
  await openScreen(page, '/home');
  await page.evaluate(() => App.setPref('confirmations', 'quiet'));
  const [id] = await firstSongs(page, 1);
  await page.evaluate((id) => App.player.addToQueue([id]), id);
  await expect(page.locator('#announcer')).toContainText('Added to queue');
  await expect(toasts(page)).toHaveCount(0);
  await page.evaluate(() => { App.toast('Something went wrong', { kind: 'error', confirms: 'queue' }); App.toast('Another device took over the music', { icon: 'connect' }); });
  await expect(toasts(page)).toHaveCount(2);
});
