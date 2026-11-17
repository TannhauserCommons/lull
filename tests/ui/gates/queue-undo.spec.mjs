// Undo of a queue edit works on the entries it touched, not on the numbers it saw: Remove's Undo puts the song back next
// to the song it sat beside, Clear's Undo brings back the old queue around the song that still plays and keeps what was
// added since, and either does nothing once the queue it belonged to is gone. docs/context/web.md, the player.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

// Plays five songs [a, b, c, d, e] of the playlist from `start` (paused); returns their ids.
async function play(page, start) {
  await openScreen(page, `/playlist/${ids.playlist}`);
  return page.evaluate((start) => {
    const t = [...document.querySelectorAll('#view li[data-track-id]')].slice(0, 7).map((r) => r.getAttribute('data-track-id'));
    App.player.playList(t.slice(0, 5), start, { autoplay: false });
    App.player.pause();
    return t;
  }, start);
}
const queue = (page) => page.evaluate(() => ({ ids: App.player.state.queue.map((q) => q.trackId), i: App.player.state.index }));
const undo = (page) => page.locator('#toasts .toast [data-toast-action]').click();

test('Undo of a removed song puts it back beside the song it sat after, whatever was added since', async ({ page }) => {
  const [a, b, c, d, e, f] = await play(page, 0);
  await page.evaluate(() => App.player.removeAt(2));                    // c
  await page.evaluate((f) => App.player.playNext(f), f);                // a f b d e
  await undo(page);
  expect((await queue(page)).ids).toEqual([a, f, b, c, d, e]);
});

test('Undo of a removed song before the playing one keeps the playing mark on the playing song', async ({ page }) => {
  const [a, b, c, d, e] = await play(page, 3);
  await page.evaluate(() => App.player.removeAt(1));                    // b
  await undo(page);
  expect(await queue(page)).toEqual({ ids: [a, b, c, d, e], i: 3 });
});

test('Undo of a removed song does nothing once its queue was replaced', async ({ page }) => {
  const [a, b, c, d, e, f, g] = await play(page, 0);
  await page.evaluate(() => App.player.removeAt(2));
  await page.evaluate(([f, g]) => App.player.playList([f, g], 0, { autoplay: false }), [f, g]);
  await undo(page);
  expect((await queue(page)).ids, 'the new queue is as it was').toEqual([f, g]);
});

test('Undo of Clear brings the old queue back around the playing song and keeps what was added since', async ({ page }) => {
  const [a, b, c, d, e, f] = await play(page, 1);
  await page.evaluate(() => App.player.clear());                        // b
  await page.evaluate((f) => App.player.addToQueue(f), f);             // b f
  await undo(page);
  expect(await queue(page)).toEqual({ ids: [a, b, f, c, d, e], i: 1 });
});

test('Undo of Clear does nothing once another song plays', async ({ page }) => {
  const [a, b, c, d, e, f] = await play(page, 1);
  await page.evaluate(() => App.player.clear());
  await page.evaluate((f) => App.player.addToQueue(f), f);
  await page.evaluate(() => App.player.next());                         // f plays
  await undo(page);
  expect(await queue(page)).toEqual({ ids: [b, f], i: 1 });
});
