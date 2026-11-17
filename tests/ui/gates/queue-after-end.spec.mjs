// After the queue has ended, what the listener adds or presses next acts on the queue as it now is: Add to queue and
// Play next start the song they added, Play reaches the first song added after the end (and starts an unchanged ended
// queue over, as before), Previous replays the last song. docs/context/web.md, the player.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

// Plays the album's first two songs, lets the second one end the queue, and returns the album's song ids.
async function endedQueue(page) {
  await openScreen(page, '/home');
  const songs = await page.evaluate(async ([album]) => {
    const t = App.data.album(album).trackIds;
    App.player.playList(t.slice(0, 2), 1, { autoplay: false });
    App.player.pause();
    App.player.next();
    return t;
  }, [ids.album]);
  await page.waitForFunction(() => App.player.state.status === 'ended');
  return songs;
}
const now = (page) => page.evaluate(() => ({ id: App.player.state.queue[App.player.state.index]?.trackId, status: App.player.state.status, i: App.player.state.index }));

test('Add to queue after the end starts the song just added, not the first one', async ({ page }) => {
  const t = await endedQueue(page);
  await page.evaluate((id) => App.player.addToQueue(id), t[2]);
  const n = await now(page);
  expect(n).toMatchObject({ id: t[2], i: 2 });
  expect(n.status).not.toBe('ended');
});

test('Play next after the end starts the song just added', async ({ page }) => {
  const t = await endedQueue(page);
  await page.evaluate((id) => App.player.playNext(id), t[2]);
  expect(await now(page)).toMatchObject({ id: t[2], i: 2 });
});

test('Play after the end starts an unchanged queue over, and reaches the first song added since', async ({ page }) => {
  const t = await endedQueue(page);
  await page.evaluate(() => App.player.play());
  expect(await now(page), 'nothing added: the queue starts over').toMatchObject({ id: t[0], i: 0 });
  await page.evaluate(() => App.player.next());
  await page.evaluate(() => App.player.next());
  await page.waitForFunction(() => App.player.state.status === 'ended');
  // a radio top-up or another device's add lands in the queue while it is over
  await page.evaluate((id) => {
    const q = App.player.state.queue;
    q.push({ ...q[0], uid: 'late', trackId: id });
  }, t[2]);
  await page.evaluate(() => App.player.play());
  expect(await now(page), 'the song added after the end').toMatchObject({ id: t[2], i: 2 });
});

test('Previous after the end replays the last song', async ({ page }) => {
  const t = await endedQueue(page);
  await page.evaluate(() => App.player.prev());
  const n = await now(page);
  expect(n).toMatchObject({ id: t[1], i: 1 });
  expect(n.status).not.toBe('ended');
});
