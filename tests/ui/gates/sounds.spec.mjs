// lull's UI sounds on the real page (ADR-0081, web/sounds.js): a browser tab nobody has tapped fetches none of them,
// the start-up sound plays once per app start and never on a reload, nothing but play and pause sounds while music plays, the buffering loop is gone the moment the song sounds,
// and nothing sounds while the page is hidden. Every sound the page starts is read from Web Audio itself (each buffer
// source started or stopped), named by the page's `sound` event.
import { expect, test as base } from '@playwright/test';
import { openScreen, ownHub } from './lib.mjs';

// a fixture hub of its own: a slow transfer is per song on a hub, and another gate's (fit.spec) must not reset this one's
const test = base.extend({
  hub: [async ({}, use) => { const hub = await ownHub(); await use(hub); hub.close(); }, { scope: 'worker' }],
  baseURL: async ({ hub }, use) => use(hub.url),
});

async function listen(page, { standalone = false } = {}) {
  await page.addInitScript((standalone) => {
    window.__sounds = []; window.__stops = []; window.__heard = [];
    const start = AudioBufferSourceNode.prototype.start, stop = AudioScheduledSourceNode.prototype.stop;
    AudioBufferSourceNode.prototype.start = function (...a) { window.__sounds.push({ loop: this.loop, at: performance.now() }); return start.apply(this, a); };
    AudioScheduledSourceNode.prototype.stop = function (...a) {
      window.__stops.push({ loop: this.loop, at: performance.now(), within: (a[0] || 0) - this.context.currentTime });
      return stop.apply(this, a);
    };
    // an installed app starts audio by itself (sounds.js: the start-up sound plays only there)
    if (standalone) {
      const mm = window.matchMedia.bind(window);
      window.matchMedia = (q) => (q === '(display-mode: standalone)' ? { matches: true, media: q, addEventListener() {}, removeEventListener() {} } : mm(q));
    }
    addEventListener('DOMContentLoaded', () => {
      window.__named = [];
      App.on('sound', (n) => window.__named.push(n));
      App.on('heard', (v) => window.__heard.push({ v, at: performance.now() }));
    });
  }, standalone);
}
const named = (page) => page.evaluate(() => window.__named.slice());

test('a browser tab nobody has tapped fetches none of the sounds, and its first tap brings them', async ({ page }) => {
  const asked = [];
  page.on('request', (r) => {
    const path = new URL(r.url()).pathname;
    if (/\/(sounds|sound-rules|eggs|egg-rules)\.js$|\/packs\//.test(path)) asked.push(path);
  });
  await listen(page);
  await openScreen(page, '/home', { clock: false });
  await page.waitForTimeout(4000);              // longer than the packs wait for the first screen to settle (1.5 s, then idle)
  expect(asked, 'fetched before a tap').toEqual([]);
  expect(await page.evaluate(() => !!window.App.sounds)).toBe(false);
  await page.mouse.click(5, 5);
  await page.waitForFunction(() => !!window.App.sounds);
  await page.waitForFunction(() => window.App.sounds.vocab(), null, { timeout: 10000 });
  expect(asked.some((p) => p.endsWith('/sounds.js')), 'the sounds came with the tap').toBe(true);
});

test('the start-up sound plays once per app start, and a reload makes no sound', async ({ page }) => {
  await listen(page, { standalone: true });
  await openScreen(page, '/home', { clock: false });
  await page.waitForFunction(() => window.__named.includes('boot'), null, { timeout: 8000 });
  await page.reload();
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await page.waitForTimeout(4000);              // longer than a start-up sound may wait (sound-rules.js BOOT_WITHIN_MS)
  expect(await named(page), 'sounds after a reload').toEqual([]);
  expect(await page.evaluate(() => window.__sounds.length), 'audio started after a reload').toBe(0);
});

test('while music plays only play and pause sound; a like sounds once the music is paused', async ({ page }) => {
  await listen(page);
  await openScreen(page, '/home', { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  await page.locator('#view .trow[data-track-id]').first().dblclick();
  await page.waitForFunction(() => App.player.heard(), null, { timeout: 15000 });
  await page.evaluate(() => { window.__named.length = 0; });
  const like = page.locator('#pbar [data-act="like"]').first();
  await like.click();
  await page.waitForTimeout(600);
  expect(await named(page), 'a like while the music plays').toEqual([]);
  await page.locator('#pbar [data-act="play"]').first().click();
  await page.waitForFunction(() => window.__named.includes('pause'));
  await page.waitForTimeout(200);
  await like.click();                            // unliked: silent
  await page.waitForTimeout(200);
  await like.click();                            // liked again, in silence
  await page.waitForFunction(() => window.__named.includes('liked'));
});

test('the buffering loop plays only in silence and is gone the moment the song sounds', async ({ page, request }) => {
  await listen(page);
  await openScreen(page, '/home', { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  const track = await page.waitForSelector('#view .trow[data-track-id]').then((r) => r.getAttribute('data-track-id'));
  // a transfer that starts slowly enough for a few seconds of silence first (the browser waits for a few hundred KB)
  await request.post('/__fixture/trickle', { data: { id: track, bytesPerS: 100000 } });
  await page.mouse.click(5, 5);                  // the listener's first tap: sounds may play from here
  await page.evaluate((id) => App.player.playTrack(id, [id]), track);
  await page.waitForFunction(() => window.__sounds.some((s) => s.loop), null, { timeout: 15000 });
  await page.waitForFunction(() => window.__heard.some((h) => h.v), null, { timeout: 20000 });
  const r = await page.evaluate(() => {
    const heard = window.__heard.find((h) => h.v).at;
    const stop = window.__stops.find((s) => s.loop && s.at >= heard - 1);
    const startedOverMusic = window.__sounds.filter((s) => s.loop).some((s) => window.__heard.some((h, i, all) => h.v && h.at <= s.at && !(all[i + 1] && all[i + 1].at <= s.at)));
    return { stopAfter: stop ? stop.at - heard : null, within: stop ? stop.within : null, startedOverMusic };
  });
  expect(r.stopAfter, 'the loop is told to stop as the song sounds (ms)').not.toBeNull();
  expect(r.stopAfter).toBeLessThan(100);
  expect(r.within, 'and fades out at once (s)').toBeLessThan(0.08);
  expect(r.startedOverMusic, 'a loop started while music played').toBe(false);
});

test('nothing sounds while the page is hidden', async ({ page }) => {
  await listen(page);
  await openScreen(page, '/home', { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  await page.mouse.click(5, 5);
  await page.evaluate(() => {
    Object.defineProperty(document, 'visibilityState', { get: () => 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    window.__named.length = 0;
  });
  const t = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, t);
  await page.locator('#view .trow [data-act="like"]').first().click({ force: true });
  await page.waitForTimeout(600);
  expect(await named(page)).toEqual([]);
});
