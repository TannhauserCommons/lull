// The same song twice in the queue (or a list) is two entries: the player tells them apart by their place, not by the
// song's id. A pick of a source stays on the playing copy, a double-click on the second copy in a list starts at the
// second copy, and only the playing entry's row in the queue is drawn playing. docs/context/web.md, the player.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

// Plays [a, b, a] (a song, another, the first again) from `start`, paused; returns [a, b].
async function playTwice(page, start) {
  await openScreen(page, '/home');
  return page.evaluate(
    ([album, start]) => {
      const [a, b] = App.data.album(album).trackIds;
      App.player.playList([a, b, a], start, { autoplay: false });
      App.player.pause();
      return [a, b];
    },
    [ids.album, start],
  );
}
const at = (page) => page.evaluate(() => ({ i: App.player.state.index, n: App.player.state.queue.length }));

test('choosing a source for the playing second copy of a song stays on that copy', async ({ page }) => {
  await playTwice(page, 2);
  await page.evaluate(() => App.player.useSource(App.player.currentId(), { peer: 'fixture-peer', path: 'x.flac', format: 'flac', sizeBytes: 1e6, speed: 1e6 }));
  expect(await at(page)).toEqual({ i: 2, n: 3 });
});

test('a double-click on the second copy of a song in a list starts at the second copy', async ({ page }) => {
  const [a, b] = await playTwice(page, 1);
  await page.evaluate(([a, b]) => {
    const list = document.createElement('ol');
    list.id = 'dup-list';
    list.setAttribute('data-ids', [a, b, a].join(','));
    list.innerHTML = [a, b, a].map((id) => `<li class="trow" data-track-id="${id}"><span>row</span></li>`).join('');
    document.body.append(list);
  }, [a, b]);
  await page.locator('#dup-list li').nth(2).dblclick();
  expect(await at(page)).toEqual({ i: 2, n: 3 });
});

test('only the playing entry of a song queued twice is drawn playing in the queue', async ({ page }) => {
  await playTwice(page, 0);
  await page.evaluate(() => App.openPanel('queue'));
  await page.waitForSelector('.ctx .now-q__row[data-uid]');
  await page.evaluate(() => App.syncRows());
  const playing = await page.$$eval('.ctx .now-q__row[data-uid]', (rows) => rows.map((r) => r.classList.contains('is-playing')));
  expect(playing).toEqual([true, false, false]);
});
