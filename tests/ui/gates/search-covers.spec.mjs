// A search's song rows show their album's cover as soon as the hub knows it (discovery.md#bare-songs), whichever list
// they are in and whenever the cover becomes known: a Soulseek song with no album in its tags that the hub finds to be
// a song it holds on a pictured album, and a song whose album gains a cover after the row was drawn. Live
// (2026-11-12, "odetari blackhole"), every Soulseek row showed lull's placeholder beside the top result's real cover.

import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

test('song rows already drawn take their album cover when the hub learns it', async ({ page, request }) => {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);   // the event stream carries it, so emits reach this page only
  const emit = (topic, data) => request.post('/__fixture/emit', { headers, data: { topic, data } });

  // an album only the catalog has (not saved, no cover known yet), and one of its songs
  const { items, entities } = await (await request.get('/__fixture/unsaved-songs?n=1', { headers })).json();
  const song = entities.tracks[items[0]];
  const album = entities.albums[song.albumId];
  expect(album.art, 'the fixture album starts without a cover').toBeUndefined();

  const q = `${song.title} ${song.artist}`.toLowerCase();   // the fixture matches songs by "title artist"
  const asked = page.waitForResponse((r) => r.url().includes('/api/soulseek/search'));
  await openScreen(page, `/search?q=${encodeURIComponent(q)}`);
  const songs = page.locator('#view .disc-songs li[data-track-id]');
  await expect(songs.first()).toBeAttached();

  // Soulseek's answer: one song no catalog knew, its tags naming no album (the artist's album "")
  const bare = { ...song, id: 't-' + 'b'.repeat(16), title: `${song.title} (peer copy)`, album: '', albumId: 'al-' + 'b'.repeat(16) };
  const searchId = 'srch-' + q.replace(/\s+/g, ' ').replace(/\W+/g, '-');
  await asked;
  await page.waitForTimeout(300);   // the fixture's own (empty) answer has gone out
  await emit('search.results', {
    searchId, query: q, files: 3, peers: 3, done: true, onlySoulseek: [bare.id],
    results: [{ kind: 'track', trackId: bare.id, sources: 3, best: { format: 'MP3 320' } }],
    entities: { tracks: { [bare.id]: bare } },
  });
  const onlyRow = page.locator(`#view .disc-only li[data-track-id="${bare.id}"]`);
  await expect(onlyRow).toBeAttached();

  const drawn = (row, id) => row.locator(`[data-art="${id}"], [data-wait-art="${id}"]`);
  const catalogRow = page.locator(`#view .disc-songs li[data-track-id="${song.id}"]`);
  await expect(drawn(catalogRow, album.id), 'no cover is known for the album yet').toHaveCount(0);
  await expect(drawn(onlyRow, album.id)).toHaveCount(0);

  // The hub learns the album's cover (an album page, a name lookup) and sends what pages drew without it, the bare
  // song now its song on that album: `catalog.tracks`, as resolve::news sends it.
  const art = `api/catalog/art/${album.id}`;
  await emit('catalog.tracks', {
    entities: {
      tracks: { [song.id]: song, [bare.id]: { ...song, id: bare.id } },
      albums: { [album.id]: { ...album, art } },
    },
  });
  await expect(drawn(catalogRow, album.id), 'the catalog row, drawn before the cover was known').toBeAttached();
  await expect(drawn(onlyRow, album.id), 'the Soulseek row, on its album now').toBeAttached();
  await expect(onlyRow.locator('.trow__title, [class*="title"]').first()).toContainText(song.title);
});
