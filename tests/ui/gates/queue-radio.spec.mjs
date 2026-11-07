// The queue always holds the radio switch. Switching radio off must leave the switch where it was, so it can be switched
// on again; a queue that never had a radio can start one from it. In every design, on a laptop (the queue panel) and on a phone
// (Now Playing, Queue tab); in Petal also Now Playing's queue card. docs/context/web.md, Radio.

import { expect, test } from '@playwright/test';
import { designs, ids, openScreen, settle } from './lib.mjs';

const sizes = {
  laptop: { viewport: { width: 1366, height: 768 } },
  card: { viewport: { width: 1340, height: 1000 } },   // Petal draws Now Playing's queue card from here, while the panel is not docked
  phone: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true },
};
const SWITCH = '[data-act="now-radio-keep"]';

// Plays the album, with or without a radio on it, and shows the queue where this size shows it: the panel on a laptop,
// Petal's queue card on Now Playing, the Queue tab on a phone. Returns the surface's selector.
async function showQueue(page, size, surface, withRadio) {
  await page.evaluate(async ([album, withRadio]) => {
    const a = App.data.album(album);
    if (withRadio) await App.player.startRadio({ kind: 'track', id: a.trackIds[0] });
    else App.player.playList(a.trackIds, 0, { at: 30, autoplay: false });
    App.player.pause();
  }, [ids.album, withRadio]);
  await page.waitForFunction(() => App.player.current());
  if (size === 'phone') {
    await page.evaluate(() => { location.hash = '#/now?tab=queue'; });
    await page.waitForSelector(`#view ${SWITCH}`);
    await settle(page);
    return '#view';
  }
  if (surface === 'card') {
    await page.evaluate(() => { location.hash = '#/now'; });
    await page.waitForSelector(`#view .now-qcard ${SWITCH}`);
    await settle(page);
    return '#view .now-qcard';
  }
  await page.evaluate(() => App.openPanel('queue'));
  await page.waitForSelector(`.ctx ${SWITCH}`);
  await settle(page);
  return '.ctx';
}

const radioOf = (page) => page.evaluate(() => { const r = App.player.state.radio; return r && { keepsAdding: r.keepsAdding, seed: r.seed }; });

for (const [size, use] of Object.entries(sizes)) {
  test.describe(size, () => {
    test.use(use);
    for (const design of designs) {
      for (const surface of size === 'phone' ? ['tab'] : size === 'card' ? (design === 'petal' ? ['card'] : []) : ['panel']) {
        test(`the queue's radio switch turns radio off and back on / ${design} / ${surface}`, async ({ page }) => {
          await openScreen(page, `/album/${ids.album}`, { design });
          const where = await showQueue(page, size, surface, true);
          const sw = page.locator(`${where} ${SWITCH}`).first();
          await expect(sw).toHaveAttribute('aria-checked', 'true');
          await expect(page.locator(`${where} .now-q__steer`).first()).toBeAttached();

          // why it stopped is told in every design, with a way to try again
          await page.evaluate(() => App.player.setRadio({ error: 'the hub did not answer' }));
          const err = page.locator(`${where} .now-q__err`).first();
          await expect(err).toBeVisible();
          await expect(err).toContainText('the hub did not answer');
          await expect(err.getByRole('button', { name: 'Try again' })).toBeVisible();
          await err.getByRole('button', { name: 'Try again' }).click();
          await expect(page.locator(`${where} .now-q__err`)).toHaveCount(0);

          await sw.click();
          await expect(sw, 'the switch is still there to switch on').toBeVisible();
          await expect(sw).toHaveAttribute('aria-checked', 'false');
          await expect(page.locator(`${where} .now-q__radio-name`).first()).toContainText('stopped');
          await expect(page.locator(`${where} .now-q__steer`), 'Steer is for a running radio').toHaveCount(0);
          expect((await radioOf(page)).keepsAdding).toBe(false);
          const seed = (await radioOf(page)).seed;

          // by keyboard: the switch keeps focus through the repaint, and one key turns the same radio back on
          await sw.focus();
          await page.keyboard.press('Space');
          await expect(sw).toHaveAttribute('aria-checked', 'true');
          await expect(sw).toBeFocused();
          expect(await radioOf(page)).toEqual({ keepsAdding: true, seed });
          await expect(page.locator(`${where} .now-q__steer`).first()).toBeAttached();
        });
      }

      // the music carries on: the queue and the playing song's place stay as they were, the radio follows the queue
      if (size !== 'card') test(`a queue that never had a radio continues into one from its switch / ${design}`, async ({ page }) => {
        await openScreen(page, `/album/${ids.album}`, { design });
        const where = await showQueue(page, size, 'panel', false);
        const sw = page.locator(`${where} ${SWITCH}`).first();
        await expect(sw).toBeVisible();
        await expect(sw).toHaveAttribute('aria-checked', 'false');
        expect(await radioOf(page)).toBeNull();
        const queueNow = () => page.evaluate(() => ({ ids: App.player.state.queue.map((q) => q.trackId), groups: App.player.state.queue.map((q) => q.group), index: App.player.state.index, pos: App.player.state.pos }));
        const before = await queueNow();
        expect(before.pos, 'a place in the song to keep').toBeGreaterThan(0);
        const playing = before.ids[before.index];

        await sw.click();
        await page.waitForFunction(() => App.player.state.radio && App.player.state.radio.keepsAdding && App.player.state.queue.some((q) => q.group === 'radio'));
        expect((await radioOf(page)).seed).toEqual({ kind: 'track', id: playing });
        const after = await queueNow();
        expect(after.ids.slice(0, before.ids.length), 'the queue is as it was, in order').toEqual(before.ids);
        expect(after.index, 'the same song is playing').toBe(before.index);
        expect(after.pos, 'and has not restarted').toBe(before.pos);
        expect(after.ids.length, 'radio songs added').toBeGreaterThan(before.ids.length);
        expect(after.groups.slice(before.ids.length).every((g) => g === 'radio'), 'after the queue, as radio').toBe(true);
        await expect(sw).toHaveAttribute('aria-checked', 'true');
        await expect(page.locator(`${where} .now-q__steer`).first()).toBeAttached();
      });
    }
  });
}
