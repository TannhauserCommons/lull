// A share page's Add to queue (docs/adr/0034; discover.js, #/open?link=…&do=queue), whichever way it reaches lull (the
// Android app, the desktop app, lull in the browser): the song, or the album's songs, join "next up" without playing or
// interrupting anything, of the listener's device playing now (Connect's enqueue) or else of this one, and lull goes
// back to the screen it showed, or Home when it opened on the link.

import { expect, test } from '@playwright/test';
import { ids, openScreen, screens } from './lib.mjs';

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

test('while another of the listener\'s devices plays, the song joins that device\'s queue', async ({ page, request }) => {
  // This test's own events on the fixture hub (server.mjs, X-Fixture-Client): its page alone hears them.
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  await openScreen(page, screens.settings);
  await request.post('/__fixture/emit', { headers, data: { topic: 'device.state', data: { deviceId: 'd-phone', trackId: null, position: 0, playing: true } } });
  await expect.poll(() => page.evaluate(() => App.connect.playingElsewhere()?.id ?? null)).toBe('d-phone');

  const sent = page.waitForRequest((r) => r.method() === 'POST' && r.url().endsWith('/api/devices/d-phone/command'));
  await page.evaluate((route) => { location.hash = route; }, queueing(SONG));
  const body = (await sent).postDataJSON();
  expect(body.action).toBe('enqueue');
  expect(body.tracks).toHaveLength(1);
  await expect(page.locator('.toast')).toContainText('Added to Phone\'s queue: Static Circuit');
  expect(await player(page)).toMatchObject({ path: screens.settings, queue: [] });
});

test('the device playing adds what another device sends it to next up, without interrupting', async ({ page, request }) => {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  await openScreen(page, `/album/${ids.album}`);
  const album = await page.evaluate((id) => App.data.album(id).trackIds, ids.album);
  await page.evaluate((list) => App.player.playList(list, 0), album.slice(0, 2));
  await expect.poll(() => player(page)).toMatchObject({ playing: true });
  const playing = await page.evaluate(() => App.player.currentId());

  const command = { deviceId: 'd-fixture', from: 'd-phone', action: 'enqueue', tracks: [album[5]] };
  await request.post('/__fixture/emit', { headers, data: { topic: 'device.command', data: command } });
  await expect.poll(async () => (await player(page)).queue).toEqual([album[0], album[5], album[1]]);
  expect(await page.evaluate(() => App.player.currentId())).toBe(playing);
  expect((await player(page)).playing).toBe(true);
});

test('an album joins whole, and lull opened on the link goes Home', async ({ page }) => {
  await openScreen(page, queueing(ALBUM));
  await expect.poll(() => player(page)).toMatchObject({ path: '/home', playing: false });
  expect((await player(page)).queue).toHaveLength(14);
});
