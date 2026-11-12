// lull's web app on a phone, with the page out of sight (web/app.js: the browser output's Web Audio graph, and Media
// Session). A listener (2026-11, an installed web app on his phone): with the app left or the screen locked the
// lock screen said the song was playing and its time ran on, but nothing sounded until he opened the app again; and
// its controls offered 10-second skips, no next or previous song. Later, in sight, the sound stopped and came back only
// with a restart of the app.
//
// The page is put out of sight by its visibility, and the system's side is played by the gate as iOS does it
// (WebKit's MediaSessionManagerIOS restricts Web Audio in the background; AudioContext::
// shouldOverrideBackgroundPlaybackRestriction lifts that only for an audio session of type playback): leaving the
// screen interrupts the page's contexts, a resume waits until the page is back, and coming back ends it. Chromium has no
// navigator.audioSession, so the gate gives it Safari's. docs/context/quality.md#ui-gates.
import { expect, test as base } from '@playwright/test';
import { openScreen, ownHub } from './lib.mjs';

// a fixture hub of its own, as sounds.spec: its songs are real audio the decks play
const test = base.extend({
  hub: [async ({}, use) => { const hub = await ownHub(); await use(hub); hub.close(); }, { scope: 'worker' }],
  baseURL: async ({ hub }, use) => use(hub.url),
});

function phone() {
  // the page's visibility, set by the gate
  let hidden = false;
  Object.defineProperty(Document.prototype, 'hidden', { configurable: true, get: () => hidden });
  Object.defineProperty(Document.prototype, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') });
  window.__show = (shown) => { hidden = !shown; document.dispatchEvent(new Event('visibilitychange')); };

  // Safari's audio session, where the browser has none
  if (!navigator.audioSession) Object.defineProperty(Navigator.prototype, 'audioSession', { configurable: true, value: { type: 'auto' } });

  // the context the decks play through (the UI sounds have their own), the elements in it, and every element played
  const source = AudioContext.prototype.createMediaElementSource, play = HTMLMediaElement.prototype.play;
  window.__routed = new WeakSet(); window.__played = new Set();
  AudioContext.prototype.createMediaElementSource = function (el) { window.__player = this; window.__routed.add(el); return source.call(this, el); };
  HTMLMediaElement.prototype.play = function () { window.__played.add(this); return play.call(this); };

  // iOS: out of sight, a context is interrupted unless the session is for playback; a resume meanwhile waits
  const contexts = new Set(), held = new Set();
  const suspend = AudioContext.prototype.suspend, resume = AudioContext.prototype.resume;
  AudioContext.prototype.resume = function () {
    contexts.add(this);
    if (held.has(this)) return new Promise(() => {});
    return resume.call(this);
  };
  const playback = () => ['playback', 'play-and-record'].includes(navigator.audioSession.type);
  document.addEventListener('visibilitychange', () => {
    const all = new Set([...contexts, window.__player].filter(Boolean));
    if (hidden && !playback()) all.forEach((c) => { if (c.state === 'running') { held.add(c); suspend.call(c); } });
    if (!hidden) held.forEach((c) => { held.delete(c); if (c.state !== 'closed') resume.call(c); });
  });
  window.__system = {
    interrupt: (c) => suspend.call(c),                          // a call, or another app's audio, while the page is in sight
    break: (c) => { held.add(c); suspend.call(c); },            // stopped for good: a resume never takes (WebKit bug 291892)
  };
}

async function playSong(page) {
  await page.addInitScript(phone);
  await openScreen(page, '/home', { clock: false });
  const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  await page.locator('#view .trow[data-track-id]').first().dblclick();
  await page.waitForFunction(() => App.player.heard() && window.__player && window.__player.state === 'running', null, { timeout: 15000 });
}
const state = (page) => page.evaluate(() => ({ context: window.__player.state, playing: App.player.state.playing, pos: App.player.state.pos }));

test('the lock screen offers the previous and next song, and no 10-second skips', async ({ page }) => {
  await page.addInitScript(() => {
    window.__actions = {};
    if (!navigator.mediaSession) return;
    const set = navigator.mediaSession.setActionHandler.bind(navigator.mediaSession);
    navigator.mediaSession.setActionHandler = (action, fn) => { window.__actions[action] = !!fn; return set(action, fn); };
  });
  await openScreen(page, '/home', { clock: false });
  test.skip(!(await page.evaluate(() => 'mediaSession' in navigator)), 'no Media Session in this browser');
  const actions = await page.evaluate(() => Object.keys(window.__actions).filter((a) => window.__actions[a]).sort());
  expect(actions).toEqual(['nexttrack', 'pause', 'play', 'previoustrack', 'seekto', 'stop']);
});

test('a song keeps sounding with the page out of sight', async ({ page }) => {
  await playSong(page);
  await page.evaluate(() => window.__show(false));
  await page.waitForTimeout(1500);
  const away = await state(page);
  expect(away.context, 'the decks\' context, out of sight').toBe('running');
  expect(away.playing).toBe(true);
  await page.evaluate(() => window.__show(true));
  expect((await state(page)).context).toBe('running');
});

test('a context the system stops while a song plays starts again; a paused song leaves it stopped', async ({ page }) => {
  await playSong(page);
  await page.evaluate(() => window.__system.interrupt(window.__player));
  await expect.poll(() => state(page).then((s) => s.context), { message: 'resumed under a playing song' }).toBe('running');
  expect((await state(page)).playing).toBe(true);

  await page.evaluate(() => App.player.pause());
  await page.waitForFunction(() => !App.player.state.playing);
  await page.evaluate(() => window.__system.interrupt(window.__player));
  await page.waitForTimeout(800);
  expect((await state(page)).context, 'nothing plays: nothing to wake for').toBe('suspended');
});

test('a song whose sound stopped for good goes on without Web Audio, without a restart', async ({ page }) => {
  const errors = [];
  page.on('pageerror', (e) => errors.push(e.message));
  await playSong(page);
  await page.evaluate(() => window.__system.break(window.__player));
  const from = (await state(page)).pos;
  // sounding again: an element out of the stopped context plays the song on from about where it was
  await page.waitForFunction(() => [...window.__played].some((el) => !el.paused && !window.__routed.has(el) && el.currentTime > 0.5),
    null, { timeout: 10000 });
  const now = await state(page);
  expect(now.playing).toBe(true);
  expect(now.pos, 'the song went on, not from the start').toBeGreaterThan(from);
  expect(now.context, 'the stopped context was let go').toBe('closed');
  await page.evaluate(() => window.__show(false));
  await page.evaluate(() => window.__show(true));
  await page.waitForTimeout(500);
  expect(errors).toEqual([]);
});
