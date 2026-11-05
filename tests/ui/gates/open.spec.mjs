// A share page's Add to queue (docs/adr/0034; discover.js, #/open?link=…&do=queue), whichever way it reaches lull (the
// Android app, the desktop app, lull in the browser): the song, or the album's songs, join "next up" without playing or
// interrupting anything, and lull goes back to the screen it showed, or Home when it opened on the link.

import { expect, test } from '@playwright/test';
import { openScreen, screens } from './lib.mjs';

// Fixture names (fixture-hub/library.mjs): lib.mjs's ids.album, and its first song.
const HOST = 'https://lull.share-host.invalid';
const ALBUM = `${HOST}/1/album/The%20Distant%20Forest/Signal%20Circuit`;
const SONG = `${HOST}/1/song/The%20Distant%20Forest/Static%20Circuit`;
const queueing = (link) => `/open?link=${encodeURIComponent(link)}&do=queue`;

const player = (page) => page.evaluate(() => ({
  path: App.current.path,
  queue: App.player.state.queue.map((q) => q.trackId),
  playing: App.player.state.playing,
}));

test('a song joins next up without playing, and lull goes back to the screen it showed', async ({ page }) => {
  await openScreen(page, screens.settings);
  await page.evaluate((route) => { location.hash = route; }, queueing(SONG));
  await expect.poll(() => player(page)).toMatchObject({ path: screens.settings, playing: false });
  const { queue } = await player(page);
  expect(queue).toHaveLength(1);
  expect(await page.evaluate(() => App.player.current().title)).toBe('Static Circuit');
  await expect(page.locator('.toast')).toContainText('Added to queue: Static Circuit');
});

test('an album joins whole, and lull opened on the link goes Home', async ({ page }) => {
  await openScreen(page, queueing(ALBUM));
  await expect.poll(() => player(page)).toMatchObject({ path: '/home', playing: false });
  expect((await player(page)).queue).toHaveLength(14);
});
