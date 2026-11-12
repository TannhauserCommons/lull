// lull installed on a phone's home screen (projects `mobile` and `mobile-webkit`): what Android and iOS read to install
// it (the manifest, its icons, the iOS tags), the shell that starts with no connection, and what the lock screen gets
// (Media Session's song, cover, position and controls, and Web Audio that keeps sounding with the screen off).
// A test marked fixme('Mnn: ...') is a known problem by its id in the mobile audit; its fix removes the mark.
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { ids } from './lib.mjs';
import { open } from './mobile.mjs';

test('the manifest installs lull as an app of its own', async ({ page }) => {
  await open(page, '/home');
  const m = await page.evaluate(() => fetch(document.querySelector('link[rel="manifest"]').href).then((r) => r.json()));
  expect(m.name).toBe('lull');
  expect(m.short_name).toBe('lull');
  expect(m.display).toBe('standalone');
  expect(m.start_url.startsWith(m.scope.replace(/\/$/, '')) || m.start_url.startsWith('./'), 'start_url inside scope').toBe(true);
  expect(m.theme_color).toMatch(/^#[0-9a-f]{6}$/i);
  expect(m.background_color).toMatch(/^#[0-9a-f]{6}$/i);
  const any = m.icons.filter((i) => !i.purpose || i.purpose.split(' ').includes('any'));
  expect(any.map((i) => i.sizes), 'icons Android asks for').toEqual(expect.arrayContaining(['192x192', '512x512']));
  expect(m.icons.some((i) => (i.purpose || '').includes('maskable')), 'a maskable icon').toBe(true);
  // every icon is the size it says, and the maskable one is full bleed (Android crops it to its own shape)
  const real = await page.evaluate(async (icons) => Promise.all(icons.map(async (i) => {
    const img = new Image(); img.src = i.src; await img.decode();
    const c = document.createElement('canvas'); c.width = img.naturalWidth; c.height = img.naturalHeight;
    const g = c.getContext('2d'); g.drawImage(img, 0, 0);
    return { sizes: i.sizes, real: `${img.naturalWidth}x${img.naturalHeight}`, purpose: i.purpose || 'any', corner: g.getImageData(0, 0, 1, 1).data[3] };
  })), m.icons);
  for (const i of real) expect(i.real, i.sizes).toBe(i.sizes);
  for (const i of real.filter((x) => x.purpose.includes('maskable'))) expect(i.corner, 'the maskable icon fills its corners').toBe(255);
});

// iOS takes the home-screen icon from apple-touch-icon and fills what is transparent with black
test('the home-screen icon on iOS is opaque and 180 px', async ({ page }) => {
  await open(page, '/home');
  const icon = await page.evaluate(async () => {
    const l = document.querySelector('link[rel="apple-touch-icon"]');
    if (!l) return null;
    const img = new Image(); img.src = l.href; await img.decode();
    const c = document.createElement('canvas'); c.width = img.naturalWidth; c.height = img.naturalHeight;
    const g = c.getContext('2d'); g.drawImage(img, 0, 0);
    const a = (x, y) => g.getImageData(x, y, 1, 1).data[3];
    return { size: `${img.naturalWidth}x${img.naturalHeight}`, corners: [a(0, 0), a(c.width - 1, 0), a(0, c.height - 1), a(c.width - 1, c.height - 1)] };
  });
  expect(icon, 'an apple-touch-icon').not.toBeNull();
  expect(icon.corners, 'no transparent corners for iOS to fill with black').toEqual([255, 255, 255, 255]);
  expect(icon.size).toBe('180x180');
});

test('iOS opens the installed app full screen, under a status bar that fits the theme', async ({ page }) => {
  await open(page, '/home');
  const meta = (n) => page.evaluate((n) => document.querySelector(`meta[name="${n}"]`)?.content ?? null, n);
  expect(await meta('apple-mobile-web-app-capable')).toBe('yes');
  expect(await meta('mobile-web-app-capable'), 'the name Chrome reads now').toBe('yes');
  expect(await meta('apple-mobile-web-app-status-bar-style'), 'the page under the status bar, as viewport-fit=cover draws it').toBe('black-translucent');
});

test('the browser\'s bar takes the theme\'s colour, day and night', async ({ page }) => {
  await open(page, '/home');
  const color = () => page.locator('meta[name="theme-color"]').getAttribute('content');
  await page.evaluate(() => App.setPref('theme', 'day'));
  const day = await color();
  await page.evaluate(() => App.setPref('theme', 'night'));
  // the theme's colour is written once night's styles apply, a frame or more after the switch (slower in WebKit on CI)
  await expect.poll(color, { timeout: 5000 }).not.toBe(day);
});

test.describe('with its service worker', () => {
  test.use({ serviceWorkers: 'allow' });

  test('the installed app starts with no connection', async ({ page, context }) => {
    await open(page, '/home');
    expect(await page.evaluate(() => navigator.serviceWorker.ready.then((r) => !!r.active)), 'the service worker is running').toBe(true);
    await page.reload();
    await page.waitForFunction(() => navigator.serviceWorker.controller);
    await context.setOffline(true);
    await page.reload();
    await page.waitForFunction(() => window.App && App.server && App.server.checked, null, { timeout: 15000 });
    await expect(page.locator('.tabbar')).toBeVisible();
    await expect(page.locator('body'), 'says it is offline').toContainText(/offline|can't reach/i);
    await context.setOffline(false);
  });

  // WebKit hands the page the worker's 'installed' after the worker has already claimed it, so a first install looked
  // like an update (app.js registerWorker): "New version ready" on the first visit
  test('a first visit offers no update', async ({ page }) => {
    await page.addInitScript(() => document.addEventListener('DOMContentLoaded', () => { window.__pwa = []; App.on('pwa', (e) => window.__pwa.push(e.kind)); }));
    await open(page, '/home');
    await page.evaluate(() => navigator.serviceWorker.ready);
    await page.waitForFunction(() => navigator.serviceWorker.controller);
    await page.waitForTimeout(500);
    expect(await page.evaluate(() => window.__pwa)).not.toContain('update');
  });
});

/* the lock screen and the OS's media controls: what Media Session is given while a song plays */
async function playing(page) {
  await page.addInitScript(() => {
    window.__ms = { actions: {}, positions: [] };
    if (!navigator.mediaSession) return;
    const ms = navigator.mediaSession, set = ms.setActionHandler.bind(ms);
    ms.setActionHandler = (a, fn) => { window.__ms.actions[a] = !!fn; try { set(a, fn); } catch { /* not in this browser */ } };
    if (ms.setPositionState) { const sp = ms.setPositionState.bind(ms); ms.setPositionState = (s) => { window.__ms.positions.push(s); sp(s); }; }
  });
  await open(page, `/album/${ids.album}`);
  test.skip(!(await page.evaluate(() => 'mediaSession' in navigator)), 'no Media Session in this build of the engine (Playwright\'s WebKit on Windows)');
  await page.locator('#view .trow[data-track-id] .trow__title').first().tap();
  await page.waitForFunction(() => App.player.current() && App.player.state.playing);
}

test('the lock screen shows the song, its cover and where it is', async ({ page }) => {
  await playing(page);
  await page.waitForFunction(() => navigator.mediaSession.metadata && /\/api\/art\//.test(navigator.mediaSession.metadata.artwork[0].src));
  const m = await page.evaluate(() => {
    const d = navigator.mediaSession.metadata, t = App.player.current();
    return { same: d.title === t.title && d.artist === t.artist && d.album === t.album, sizes: d.artwork.map((a) => a.sizes), state: navigator.mediaSession.playbackState, positions: window.__ms.positions.length };
  });
  expect(m.same, 'title, artist and album of the playing song').toBe(true);
  expect(m.sizes).toEqual(['96x96', '256x256', '512x512']);
  expect(m.state).toBe('playing');
  expect(m.positions, 'the lock screen\'s scrubber has the song\'s position').toBeGreaterThan(0);
});

// a listener's report (2026-11): the lock screen offered 10-second skips, and no next or previous song
test('the lock screen skips songs, not 10 seconds', async ({ page }) => {
  test.fixme(true, 'M02: app.js wireMediaSession sets seekbackward and seekforward, which iOS shows instead of previous and next');
  await playing(page);
  const set = await page.evaluate(() => Object.keys(window.__ms.actions).filter((a) => window.__ms.actions[a]).sort());
  expect(set).toEqual(expect.arrayContaining(['nexttrack', 'pause', 'play', 'previoustrack', 'seekto']));
  expect(set, 'iOS puts these where next and previous go').not.toContain('seekbackward');
  expect(set).not.toContain('seekforward');
});

// a listener's report (2026-11): with the screen locked, the lock screen counted on and nothing sounded. iOS stops a
// page's Web Audio in the background unless the page's audio session is for playback (Safari's navigator.audioSession)
// (Chromium has no audioSession: the gate gives it Safari's, set to the default 'auto')
test('a song played through Web Audio is a playback session, so it sounds with the screen off', async ({ page }) => {
  test.fixme(true, 'M01: the Web Audio graph (app.js ensureGraph) is built with the audio session left at auto');
  await page.addInitScript(() => {
    if (!window.AudioContext) return;
    if (!navigator.audioSession) Object.defineProperty(Navigator.prototype, 'audioSession', { configurable: true, value: { type: 'auto' } });
    const source = AudioContext.prototype.createMediaElementSource;
    AudioContext.prototype.createMediaElementSource = function (el) { window.__graph = this; return source.call(this, el); };
  });
  await open(page, `/album/${ids.album}`);
  test.skip(!(await page.evaluate(() => !!window.AudioContext)), 'no Web Audio in this build of the engine (Playwright\'s WebKit on Windows)');
  await page.evaluate(() => App.setPlayback({ crossfade: 4 }));   // a crossfade plays through Web Audio
  await page.locator('#view .trow[data-track-id] .trow__title').first().tap();
  await page.waitForFunction(() => App.player.current() && App.player.state.playing && window.__graph);
  expect(await page.evaluate(() => navigator.audioSession.type)).toBe('playback');
});
