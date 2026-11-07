// A song played from Home comes with its first album and moves to the catalog's once a lookup runs (discovery.md#bare-songs).
// Whatever merge carries the move, what shows the song draws it again: the queue panel as well as the player bar.

import { expect, test } from '@playwright/test';
import { ids, openScreen, settle } from './lib.mjs';

test('a queued song moved to another album shows its new cover in the queue panel and the player bar', async ({ page }) => {
  await openScreen(page, `/playlist/${ids.playlist}`);
  await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view li[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
  await page.evaluate(() => App.openPanel('queue'));
  await settle(page);
  const [playing, queued] = await page.evaluate(() => App.player.state.queue.slice(0, 2).map((q) => q.trackId));
  const album = `al-${'0'.repeat(16)}`;
  // a cover is drawn for its album's key: the picture itself once the hub hands it out, until then a place for it
  const drawnFor = (scope) => page.locator(`${scope} [data-art="${album}"], ${scope} [data-wait-art="${album}"]`).first();
  const row = (id) => `.ctx .now-q__row[data-track-id="${id}"]`;
  const move = (id) =>
    page.evaluate(
      ([id, album]) => {
        const a = { id: album, title: 'Moved here', artist: 'Velvet Orchard', artistId: 'ar-x', art: `api/catalog/art/${album}`, trackIds: [id] };
        App.data.absorb({ albums: { [album]: a }, tracks: { [id]: { ...App.data.track(id), albumId: album } } });
      },
      [id, album],
    );

  await expect(drawnFor(row(queued))).toHaveCount(0);
  await move(queued);
  await expect(drawnFor(row(queued)), 'the queued song, in the panel').toBeAttached();

  await move(playing);
  await expect(drawnFor(row(playing)), 'the playing song, in the panel').toBeAttached();
  await expect(drawnFor('#pbar'), 'and in the player bar').toBeAttached();
});
