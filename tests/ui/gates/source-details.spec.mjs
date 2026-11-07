// Source details of a song streaming from Soulseek before the hub has named a peer: the dialog, the player bar and Now
// Playing all describe the copy that plays (LullRules.playingCopy), so none says "library" of a song that isn't, and none
// draws a quality chip with nothing in it. It was read as "Playing from your library via Jellyfin", a blank chip, "saved,
// plays at once" and an empty Format, with Save still offered. The hub's stream events are held back, so no peer is known:
// the song plays from its own buffer. docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, openScreen } from './lib.mjs';

/* every chip on the page with nothing in it, wherever it is drawn */
const emptyChips = () => [...document.querySelectorAll('.q-chip')].filter((c) => !c.textContent.trim()).length;

async function streamWithoutAPeer(page, request) {
  await page.route('**/api/events*', (route) => route.abort());
  await openScreen(page, '/home', { clock: false });
  const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  const track = await page.waitForSelector('#view .trow[data-track-id]').then((r) => r.getAttribute('data-track-id'));
  await request.post('/__fixture/trickle', { data: { id: track, bytesPerS: 60000 } });
  await page.evaluate((id) => App.player.playTrack(id, [id]), track);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.received > 0, null, { timeout: 20000 });
  return track;
}

for (const design of designs) {
  test(`source details of a stream no peer has answered for / ${design}`, async ({ page, request }) => {
    await page.addInitScript((d) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: 'day' })), design);
    await streamWithoutAPeer(page, request);
    expect(await page.evaluate(() => App.player.state.peer), 'the hub named no peer').toBeNull();
    await page.evaluate(() => App.hooks.sourceDetails());
    const sheet = page.locator('.disc-sheet');
    await expect(sheet).toBeVisible();
    const text = await sheet.innerText();
    expect(text, 'it names Soulseek').toMatch(/soulseek/i);
    expect(text, 'it says a peer is still being found').toMatch(/connecting to a peer/i);
    expect(text, 'it does not call the song the library\'s').not.toMatch(/library|jellyfin|music folder|saved, plays at once/i);
    expect(await sheet.locator('.source-card__stat').innerText(), 'and how much has arrived').toMatch(/\d+% arrived/);
    expect(await page.evaluate(emptyChips), 'no chip is empty, in the dialog or the player bar').toBe(0);
    await page.evaluate(() => { App.closeTop(); location.hash = '#/now'; });
    await page.waitForFunction(() => App.current.path === '/now');
    expect(await page.evaluate(emptyChips), 'nor on Now Playing').toBe(0);
    expect(await page.locator('.main').innerText(), 'which does not say jellyfin either').not.toMatch(/jellyfin/i);
  });
}
