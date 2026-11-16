// lull's UI sounds on the real page (ADR-0081, web/sounds.js): a browser tab nobody has tapped fetches none of them,
// the start-up sound plays once per app start and never on a reload, nothing but play and pause sounds while music plays, the buffering loop is gone the moment the song sounds,
// and nothing sounds while the page is hidden. Every sound the page starts is read from Web Audio itself (each buffer
// source started or stopped), named by the page's `sound` event.
import { expect, test as base } from '@playwright/test';
import * as L from '../fixture-hub/library.mjs';
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

// The buffering loop plays while the listener wants the song, nothing sounds and the status is finding, queued or
// buffering (ADR-0041 keeps that rule, read from the view): here through the silence before a slow transfer's first
// bytes play. It is gone, fading out at once, the moment the song sounds, and never starts over music.
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

// The same rule through what the hub says of a found peer: its first bytes still to come (`buffering`, after `finding`),
// and the peer it falls back to from a stalled one (`fallback`, which the player shows as buffering), with nothing
// sounding. The loop plays through both.
test('the buffering loop plays on while a found peer\'s first bytes are awaited and through a fallback, in silence', async ({ page, request }) => {
  const songs = L.albums[L.albumIds[141]].trackIds, [found, fell] = [songs[1], songs[2]];
  await listen(page);
  for (const id of [found, fell]) await page.route((url) => url.pathname === `/api/stream/${id}`, () => {});   // nothing arrives: silence throughout
  await openScreen(page, `/album/${L.tracks[found].albumId}`, { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  await page.mouse.click(5, 5);                  // the listener's first tap: sounds may play from here
  const say = (id, status, more = {}) => request.post('/__fixture/emit', { data: { topic: 'stream', data: {
    trackId: id, jobId: 'wait-' + id, attempt: 0, received: 0, size: 0, peersFound: 3, failedPeers: [], status, ...more } } });
  const looping = () => page.evaluate(() => window.__sounds.filter((s) => s.loop).length > window.__stops.filter((s) => s.loop).length);
  const loopStops = () => page.evaluate(() => window.__stops.filter((s) => s.loop).length);

  await page.evaluate((id) => App.player.playList([id], 0), found);
  await say(found, 'finding');
  await expect.poll(looping, { message: 'the loop, while a source is found', timeout: 15000 }).toBe(true);
  const n = await loopStops();
  await say(found, 'buffering', { peer: 'mira' });
  await page.waitForFunction(() => App.player.state.status === 'buffering' && App.player.state.peer && App.player.state.peer.name === 'mira', null, { timeout: 10000 });
  await page.waitForTimeout(1500);
  expect.soft(await page.evaluate(() => App.player.heard()), 'a found peer\'s first bytes awaited: nothing sounds').toBe(false);
  expect.soft(await loopStops() - n, 'a found peer\'s first bytes awaited: times the loop was stopped').toBe(0);
  expect.soft(await looping(), 'a found peer\'s first bytes awaited: the loop plays').toBe(true);

  await page.evaluate((id) => App.player.playList([id], 0), fell);
  await say(fell, 'stalled', { peer: 'mira' });
  await page.waitForFunction(() => App.player.state.status === 'stalled', null, { timeout: 10000 });
  await say(fell, 'fallback', { peer: 'nova', attempt: 1, failedPeers: ['mira'] });
  await page.waitForFunction(() => App.player.state.fallback && App.player.state.status === 'buffering', null, { timeout: 10000 });
  await expect.poll(looping, { message: 'the loop, through the fallback in silence', timeout: 5000 }).toBe(true);
  await page.waitForTimeout(1500);
  expect.soft(await page.evaluate(() => App.player.heard()), 'the fallback: nothing sounds').toBe(false);
  expect.soft(await looping(), 'the fallback: the loop still plays').toBe(true);
});

// A song that sounded, then fell back to another peer whose bytes are still to come, is silent on a status of the rule's
// (buffering) once its sound stops, however many of the output's `time` events came between the hub's words: the loop
// comes in at once (the round-3 review's R1).
const soundsAt = (page) => page.evaluate(() => ({ heard: window.__heard.length, sounds: window.__sounds.length }));
const streamSays = (request, id, job) => (status, more = {}) => request.post('/__fixture/emit', { data: { topic: 'stream', data: {
  trackId: id, jobId: job + '-' + id, attempt: 0, received: 0, size: 4000000, peersFound: 3, peer: 'mira', failedPeers: [], status, ...more } } });
/* a song's stream: it comes, and never again once `held` is set */
async function holdable(page, id) {
  const s = { held: false };
  await page.route((url) => url.pathname === `/api/stream/${id}`, async (route) => { if (!s.held) await route.continue(); });
  return s;
}
/* the sounds loaded and the listener's first tap: sounds may play from here */
async function ready(page, route) {
  await openScreen(page, route, { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  await page.mouse.click(5, 5);
}
const sounding = (page, id) => page.waitForFunction((id) => App.player.current().id === id && App.player.heard() && App.player.state.pos > 0.3, id, { timeout: 20000 });

test('the buffering loop comes in as a song that sounded falls silent on a fallback\'s new peer, time events between', async ({ page, request }) => {
  const song = L.albums[L.albumIds[142]].trackIds[0];
  await listen(page);
  const stream = await holdable(page, song);
  await ready(page, `/album/${L.tracks[song].albumId}`);
  const say = streamSays(request, song, 'fell');
  await page.evaluate((id) => App.player.playList([id], 0), song);
  await say('streaming', { attempt: 1, received: 400000 });
  await sounding(page, song);
  // the peer stalls and the hub falls back while what arrived still plays: the output's time events come between
  await say('stalled', { attempt: 1, received: 400000 });
  await page.waitForTimeout(800);
  await say('fallback', { attempt: 1, received: 400000, failedPeers: ['mira'] });
  await page.waitForTimeout(800);
  expect(await page.evaluate(() => App.player.heard()), 'the song still sounds through the fallback').toBe(true);
  // the new peer's attempt: the page reopens the song on it, and its bytes never come
  stream.held = true;
  const at = await soundsAt(page);
  await say('queued', { attempt: 2, peer: 'nova', failedPeers: ['mira'] });
  await page.waitForFunction((n) => window.__heard.slice(n).some((h) => !h.v), at.heard, { timeout: 10000 });
  await page.waitForTimeout(2500);
  const r = await page.evaluate((at) => {
    const silent = window.__heard.slice(at.heard).find((h) => !h.v).at, loop = window.__sounds.slice(at.sounds).find((s) => s.loop);
    return { after: loop ? loop.at - silent : null, heard: App.player.heard(), status: App.player.state.status };
  }, at);
  expect(r.heard, 'the song is silent on the new peer').toBe(false);
  expect(r.after, `the loop starts after the sound stops (ms; status ${r.status})`).not.toBeNull();
  expect(r.after).toBeGreaterThan(-1);
  expect(r.after, 'and within a moment of it (ms)').toBeLessThan(1500);
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
