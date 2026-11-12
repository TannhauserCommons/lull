// The mini player's seek bar and times move while the song plays. The owner's listener (desktop app 0.2.10): "if he's
// using the mini player, the seek doesn't update till he pauses". Main sent the mini player a snapshot only from the
// page's 'tick', which the player never emits while its window is hidden (minimised, or in the tray: the mini player
// is what the listener is using then), so the position reached the mini player only when the song's state changed:
// on pause. Now main watches the clock itself while a mini player is open and the mini player moves the position on
// between snapshots. docs/context/quality.md#ui-gates.

import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

const bridge = fileURLToPath(new URL('./desktop-bridge.js', import.meta.url));

test('main keeps the mini player\'s position moving while its own window is hidden', async ({ page }) => {
  await page.addInitScript({ path: bridge });
  await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
  // the main window minimised: the page is hidden, as the tray or the taskbar leaves it
  await page.addInitScript(() => {
    Object.defineProperty(document, 'hidden', { get: () => true });
    window.__snaps = [];
    const t = window.__TAURI__, emit = t.event.emit;
    t.event.emit = (name, p) => { if (name === 'player') window.__snaps.push({ at: Date.now(), pos: p.pos, playing: p.playing }); return emit(name, p); };
  });
  await openScreen(page, '/search?q=velvet', { clock: false });
  await page.waitForSelector('#view .trow[data-track-id]');
  await page.evaluate(() => { const id = document.querySelector('#view .trow[data-track-id]').getAttribute('data-track-id'); App.player.playTrack(id, [id]); });
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.3);
  await page.evaluate(() => window.__bridge.fire('mini', { action: 'hello' }));   // the mini player opens
  await page.waitForTimeout(4500);
  const r = await page.evaluate(() => {
    const snaps = window.__snaps.filter((s) => s.playing), last = snaps[snaps.length - 1], now = Date.now();
    return { n: window.__snaps.length, real: App.player.state.pos, shown: last ? last.pos + (now - last.at) / 1000 : null };
  });
  // where the mini player's own clock puts the song, from the last thing main said, against where the song really is
  expect(r.shown, 'main said where the song was').not.toBeNull();
  expect(Math.abs(r.shown - r.real), 'the mini player\'s place, against the song\'s').toBeLessThan(1.5);
  expect(r.real, 'the song played on').toBeGreaterThan(3);
  expect(r.n, 'snapshots in 4.5 s of playing: the clock does the rest').toBeLessThanOrEqual(3);
});

test('the mini player moves the position on between snapshots', async ({ page }) => {
  await page.addInitScript({ path: bridge });
  await page.route('**/api/**', (r) => r.fulfill({ status: 404, body: 'not here' }));
  await page.goto('/desktop/wrapper.html#view=mini&design=lull&theme=day&opaque=1');
  await page.waitForFunction(() => document.querySelector('[data-dk-player]'));
  await page.evaluate(() => window.__bridge.fire('player', { track: { id: 't1', title: 'Song', artist: 'Artist', format: 'FLAC', lossless: true },
    playing: true, liked: false, pos: 20, duration: 200, durationKnown: true, buffered: [[0, 1]], received: 1, shuffle: false, repeat: 'off',
    volume: .5, status: 'streaming' }));
  const read = () => page.evaluate(() => ({ time: document.querySelector('[data-dk-pos]').textContent,
    now: +document.querySelector('.wave').getAttribute('aria-valuenow') }));
  expect(await read()).toMatchObject({ time: '0:20' });
  await page.waitForTimeout(3300);
  const moved = await read();
  expect(moved.time, 'the time, three seconds on, with no new snapshot').toBe('0:23');
  // a pause stops it
  await page.evaluate(() => window.__bridge.fire('player', { track: { id: 't1', title: 'Song', artist: 'Artist', format: 'FLAC', lossless: true },
    playing: false, liked: false, pos: 23, duration: 200, durationKnown: true, buffered: [[0, 1]], received: 1, shuffle: false, repeat: 'off',
    volume: .5, status: 'streaming' }));
  await page.waitForTimeout(2200);
  expect((await read()).time).toBe('0:23');
});
