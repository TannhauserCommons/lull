// A list hands the page an album or a playlist's ids without the songs themselves (a tile on Library's albums, Playlists'
// first four songs a playlist). The player can't load a song it doesn't know, so an album or playlist queued or played
// from a tile must have its songs read first: a listener (2026-11) got "Added 14 songs", and nothing showed or played.
import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

/** The queue as the player holds it: its length, the entries whose song the page doesn't know, and the playing status. */
const queue = (page) => page.evaluate(() => {
  const ps = App.player.state;
  return { length: ps.queue.length, unknown: ps.queue.filter((e) => !App.data.track(e.trackId)).length, status: ps.status };
});

const choose = (page, label) => page.locator('.menu__item', { hasText: new RegExp(`^${label}$`) }).click();

/** An album tile whose songs the page has not been given. */
async function bareAlbum(page) {
  const id = await page.evaluate(() => [...document.querySelectorAll('.lib-tile[data-album-id]')].map((t) => t.getAttribute('data-album-id'))
    .find((a) => App.data.album(a).trackIds.length && App.data.album(a).trackIds.some((t) => !App.data.track(t))));
  expect(id, 'an album tile with unread songs').toBeTruthy();
  return id;
}

for (const how of ['Play next', 'Add to queue']) {
  test(`an album tile's "${how}" queues songs the page had not read, and they load`, async ({ page }) => {
    await openScreen(page, '/library/albums');
    const id = await bareAlbum(page);
    const total = await page.evaluate((a) => App.data.album(a).trackIds.length, id);
    await page.locator(`[data-album-id="${id}"] a[href^="#/album/"]`).last().click({ button: 'right' });
    await choose(page, how);
    await expect.poll(() => queue(page)).toMatchObject({ length: total, unknown: 0 });
    expect((await queue(page)).status).not.toBe('idle');
  });

  test(`a playlist tile's "${how}" queues songs the page had not read, and they load`, async ({ page }) => {
    await openScreen(page, '/playlists');
    const unread = await page.evaluate((p) => App.store.get('playlists').find((x) => x.id === p).items.some((i) => !App.data.track(i.trackId)), ids.playlist);
    expect(unread, 'a playlist whose songs are unread').toBe(true);
    await page.locator(`[data-place="playlist:${ids.playlist}"] .tile__title a`).click({ button: 'right' });
    await choose(page, how);
    await expect.poll(() => queue(page)).toMatchObject({ length: 60, unknown: 0 });
    expect((await queue(page)).status).not.toBe('idle');
  });
}

test("a playlist tile's Play button plays songs the page had not read", async ({ page }) => {
  await openScreen(page, '/playlists');
  await page.locator(`[data-act="play-playlist"][data-id="${ids.playlist}"]`).first().click({ force: true });
  await expect.poll(() => queue(page)).toMatchObject({ length: 60, unknown: 0 });
  await page.evaluate(() => App.player.next());
  expect((await queue(page)).status).not.toBe('idle');
});

for (const how of ['Play next', 'Add to queue']) {
  test(`an album with no songs says so on "${how}" and queues nothing`, async ({ page }) => {
    const empty = { id: 'al-empty', title: 'Empty', artist: 'Nobody', artistId: 'ar-none', trackIds: [] };
    await page.route('**/api/albums/al-empty', (route) => route.fulfill({ json: { ...empty, entities: { albums: { 'al-empty': empty } } } }));
    await openScreen(page, '/home');
    await page.evaluate((a) => { App.data.absorb({ albums: { [a.id]: a } }); App.itemMenus.album(a.id, document.querySelector('#view')); }, empty);
    await choose(page, how);
    await expect(page.locator('.toast', { hasText: 'This album has no songs to play yet' })).toBeVisible();
    await expect(page.locator('.toast', { hasText: /Playing next|Added/ })).toHaveCount(0);
    expect((await queue(page)).length).toBe(0);
  });
}

// A Soulseek album lull has never opened answers `partial: true` with no songs and sends `album.updated` when the
// tracklist arrives, within 90 s (api.md): the songs are coming, so the click waits for them and queues them then.
for (const how of ['Play next', 'Add to queue']) {
  test(`an album whose tracklist arrives late: "${how}" waits for it and queues the songs after the playing one`, async ({ page, request }) => {
    const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
    await page.context().setExtraHTTPHeaders(headers);
    await request.post('/__fixture/album-late', { data: { id: ids.album, ms: 1500 }, headers });
    await openScreen(page, '/home');
    const playing = await page.evaluate(async () => {
      const r = await App.data.api('/__fixture/unsaved-songs?n=1'); App.data.absorb(r.entities);
      App.player.playList(r.items, 0, { autoplay: false });
      return r.items[0];
    });
    await page.evaluate((id) => {
      App.data.absorb({ albums: { [id]: { id, title: 'Late', artist: 'Soft Choir', artistId: 'ar-none', trackIds: [] } } });
      App.itemMenus.album(id, document.querySelector('#view'));
    }, ids.album);
    await choose(page, how);
    await expect(page.locator('.toast', { hasText: 'Getting the tracklist' })).toBeVisible();
    await expect.poll(() => page.evaluate(() => App.player.state.queue.length), { timeout: 10_000 }).toBeGreaterThan(1);
    const q = await page.evaluate(() => App.player.state.queue.map((e) => ({ id: e.trackId, known: !!App.data.track(e.trackId) })));
    expect(q[0].id).toBe(playing);
    expect(q.every((e) => e.known)).toBe(true);
    expect(q.slice(1).map((e) => e.id)).toEqual(await page.evaluate((id) => App.data.album(id).trackIds, ids.album));
    await expect(page.locator('.toast', { hasText: 'no songs' })).toHaveCount(0);
  });
}
