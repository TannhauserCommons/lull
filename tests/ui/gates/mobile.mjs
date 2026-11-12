// Shared by the mobile gates (mobile-*.spec.mjs, projects `mobile` and `mobile-webkit`): a screen opened as on a phone,
// a song in the player, moving between screens as a tap does, and the safe-area insets of a phone with a notch.

import { ids, openScreen, settle } from './lib.mjs';

export const isWebkit = (browser) => browser.browserType().name() === 'webkit';

/* openScreen, with the one-time install offer already made: its note would stand over what a gate measures. (The
   projects block service workers, whose first install WebKit reports as an update, M14; the gates about them allow them) */
export async function open(page, route, opts = {}) {
  await page.addInitScript(() => localStorage.setItem('lull:installOffered', 'true'));
  await openScreen(page, route, { clock: false, ...opts });
}

/* `route` open in `design`, with the album's songs in the player, paused: the mini player and Now Playing have a song */
export async function withSong(page, design, route = `/album/${ids.album}`) {
  await open(page, `/album/${ids.album}`, { design });
  await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows, { autoplay: false });
  });
  await page.waitForFunction(() => App.player.current());
  if (route !== `/album/${ids.album}`) await go(page, route);
}

/* another screen, as a tap on a link reaches it, once it has settled (Now Playing and sheets slide up) */
export async function go(page, route) {
  await page.evaluate((r) => { location.hash = '#' + r; }, route);
  await settle(page);
  await page.waitForTimeout(400);
}

/* Chromium takes a notch's insets from the DevTools protocol; WebKit can't be given any from a test */
export async function insets(page, sides) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setSafeAreaInsetsOverride', { insets: { top: 0, bottom: 0, left: 0, right: 0, ...sides } });
}
