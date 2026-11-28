// The mini player's seek bar and times move while the song plays. The owner's listener (desktop app 0.2.10): "if he's
// using the mini player, the seek doesn't update till he pauses". Main sent the mini player a snapshot only from the
// page's 'tick', which the player never emits while its window is hidden (minimised, or in the tray: the mini player
// is what the listener is using then), so the position reached the mini player only when the song's state changed:
// on pause. Now main publishes the player's view (ADR-0041) when it changes, hidden or not, and the view carries a
// clock ({ pos, at, rate }) the mini player places the song by between views. docs/context/quality.md#ui-gates.

import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';
import { fireView } from './player-windows.mjs';

const bridge = fileURLToPath(new URL('./desktop-bridge.js', import.meta.url));

test('main keeps the mini player\'s position moving while its own window is hidden', async ({ page }) => {
  await page.addInitScript({ path: bridge });
  await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
  // the main window minimised: the page is hidden, as the tray or the taskbar leaves it
  await page.addInitScript(() => {
    Object.defineProperty(document, 'hidden', { get: () => true });
    Object.defineProperty(document, 'visibilityState', { get: () => 'hidden' });
    window.__views = [];
    const t = window.__TAURI__, emit = t.event.emit, emitTo = t.event.emitTo;
    const keep = (name, p) => { if (name === 'player') window.__views.push(p); };
    t.event.emit = (name, p) => { keep(name, p); return emit(name, p); };
    t.event.emitTo = (to, name, p) => { keep(name, p); return emitTo(to, name, p); };
  });
  await openScreen(page, '/search?q=velvet', { clock: false });
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));
  await page.waitForSelector('#view .trow[data-track-id]');
  await page.evaluate(() => { const id = document.querySelector('#view .trow[data-track-id]').getAttribute('data-track-id'); App.player.playTrack(id, [id]); });
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.3);
  // the mini player opens, at the version main has
  await page.evaluate(() => window.__bridge.fire('mini', { action: 'hello', v: window.PlayerView && PlayerView.VERSION }));
  await page.waitForTimeout(4500);   // what is checked is how few views 4.5 s of playing take: the time is the test's
  const r = await page.evaluate(() => {
    const running = window.__views.filter((v) => v && v.clock && v.clock.rate === 1), last = running[running.length - 1];
    return { n: window.__views.length, real: App.player.state.pos, shown: last && window.PlayerView ? PlayerView.posAt(last, Date.now()) : null };
  });
  // where the mini player's own clock puts the song, from the last thing main said, against where the song really is
  expect(r.shown, 'main said where the song was').not.toBeNull();
  expect(Math.abs(r.shown - r.real), 'the mini player\'s place, against the song\'s').toBeLessThan(1.5);
  expect(r.real, 'the song played on').toBeGreaterThan(3);
  expect(r.n, 'views in 4.5 s of playing: the clock does the rest').toBeLessThanOrEqual(3);
});

test('the mini player moves the position on between views', async ({ page }) => {
  await page.addInitScript({ path: bridge });
  await page.route('**/api/**', (r) => r.fulfill({ status: 404, body: 'not here' }));
  await page.goto('/desktop/wrapper.html#view=mini&design=lull&theme=day&opaque=1');
  await page.waitForFunction(() => document.querySelector('[data-dk-player]'));
  const sent = await page.evaluate((f) => { const at = Date.now(); eval(f)({ clock: { at } }); return at; }, `(${fireView})`);   // playing at 0:20
  // the time it shows, and the whole seconds the window's own clock has run since the view was sent
  const read = () => page.evaluate((sent) => ({ time: document.querySelector('[data-dk-pos]').textContent,
    now: +document.querySelector('.wave').getAttribute('aria-valuenow'), ran: Math.floor((Date.now() - sent) / 1000) }), sent);
  expect(await read()).toMatchObject({ time: '0:20' });
  await page.waitForTimeout(3300);   // time passing is what is checked
  const moved = await read(), due = (s) => `0:${String(20 + s).padStart(2, '0')}`;
  expect(moved.ran, 'seconds on').toBeGreaterThanOrEqual(3);
  expect([due(moved.ran), due(moved.ran - 1)], 'the time, three seconds on and more, with no new view').toContain(moved.time);
  // a pause stops it
  await page.evaluate(fireView, { activity: 'paused', clock: { pos: 23, rate: 0 } });
  await page.waitForTimeout(2200);   // what is checked is that the time stands: two seconds and more for it to move in
  expect((await read()).time).toBe('0:23');
});
