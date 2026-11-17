// At the end of a radio queue Next waits for the radio's top-up. Two quick presses while it runs are one press: the
// first radio song plays, not the second. docs/context/web.md, Radio.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

test('two quick Next presses at the end of a radio queue move to the first new song once', async ({ page }) => {
  await openScreen(page, '/home');
  let release;
  const held = new Promise((resolve) => { release = resolve; });
  // the hub answers the radio's top-up only when the test lets it
  await page.route('**/api/radio?*', async (route) => {
    await held;
    await route.continue();
  });
  const last = await page.evaluate((album) => {
    const t = App.data.album(album).trackIds.slice(0, 2);
    // the radio is looking for songs behind the last one, as it does when the queue is about to end
    App.player.playList(t, 1, { autoplay: false, radio: { name: 'Radio', seed: { kind: 'track', id: t[0] }, keepsAdding: true, steer: 0.4 } });
    App.player.pause();
    App.player.next();
    App.player.next();
    return App.player.state.index;
  }, ids.album);
  expect(last).toBe(1);
  release();
  await page.waitForFunction(() => App.player.state.queue.length > 2 && App.player.state.index >= 2);
  expect((await page.evaluate(() => App.player.state.index)), 'the first radio song, once').toBe(2);
});
