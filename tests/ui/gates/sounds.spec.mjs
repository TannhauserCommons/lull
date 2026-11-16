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

// The buffering loop is for a wait for a source (ADR-0041): a song still being found (finding, queued) or a stalled peer
// with nothing sounding. It is gone, fading out at once, the moment the song sounds, and never starts over music: a
// transfer still arriving while the song is heard makes none. Each wait is the hub's own word (a `stream` event) for a
// song whose stream the page asks for and doesn't get yet, so nothing but the wait decides.
test('the buffering loop plays while a source is found or a stalled peer leaves silence, and is gone the moment the song sounds', async ({ page, request }) => {
  await listen(page);
  await openScreen(page, '/home', { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  await page.waitForSelector('#view .trow[data-track-id]');
  const [found, stalled, heard] = await page.evaluate(() => [...document.querySelectorAll('#view .trow[data-track-id]')].slice(0, 3).map((r) => r.getAttribute('data-track-id')));
  const holds = new Map();   // a song's stream, held until the test lets it come
  for (const id of [found, stalled]) {
    let go; holds.set(id, new Promise((r) => { go = r; })); holds.get(id).go = go;
    await page.route((url) => url.pathname === `/api/stream/${id}`, async (route) => { await holds.get(id); await route.continue(); });
  }
  const say = (id, status, more = {}) => request.post('/__fixture/emit', { data: { topic: 'stream', data: {
    trackId: id, jobId: 'wait-' + id, attempt: 0, received: 0, size: 0, peersFound: 3, failedPeers: [], status, ...more } } });
  /* the loop plays now: started more often than stopped */
  const looping = () => page.evaluate(() => window.__sounds.filter((s) => s.loop).length > window.__stops.filter((s) => s.loop).length);
  await page.mouse.click(5, 5);                  // the listener's first tap: sounds may play from here

  // finding a source: the loop plays, and goes the moment the song sounds
  await page.evaluate((id) => App.player.playTrack(id, [id]), found);
  await say(found, 'finding');
  await expect.poll(looping, { message: 'the loop, while a source is found', timeout: 15000 }).toBe(true);
  const quiet = await page.evaluate(() => window.__heard.length);
  holds.get(found).go();
  await page.waitForFunction((n) => window.__heard.slice(n).some((h) => h.v), quiet, { timeout: 20000 });
  const r = await page.evaluate((n) => {
    const heard = window.__heard.slice(n).find((h) => h.v).at;
    const stop = window.__stops.find((s) => s.loop && s.at >= heard - 1);
    return { stopAfter: stop ? stop.at - heard : null, within: stop ? stop.within : null };
  }, quiet);
  expect(r.stopAfter, 'the loop is told to stop as the song sounds (ms)').not.toBeNull();
  expect(r.stopAfter).toBeLessThan(100);
  expect(r.within, 'and fades out at once (s)').toBeLessThan(0.08);

  // a stalled peer with nothing sounding: the loop plays
  await page.evaluate((id) => App.player.playTrack(id, [id]), stalled);
  await say(stalled, 'stalled', { peer: 'mira' });
  await page.waitForFunction(() => App.player.state.status === 'stalled', null, { timeout: 10000 });
  await expect.soft.poll(looping, { message: 'the loop, while a stalled peer leaves silence', timeout: 5000 }).toBe(true);
  holds.get(stalled).go();

  // a transfer still arriving while the song is heard (three times the song's rate): no loop over the music
  await request.post('/__fixture/trickle', { data: { id: heard, bytesPerS: 24000 } });
  await page.evaluate((id) => App.player.playTrack(id, [id]), heard);
  await page.waitForFunction((id) => App.player.current().id === id && App.player.heard() && App.player.state.pos > 0.3, heard, { timeout: 20000 });
  const from = await page.evaluate(() => performance.now());
  await page.waitForTimeout(3000);
  const over = await page.evaluate((from) => window.__sounds.filter((s) => s.loop && s.at >= from).length, from);
  expect.soft(over, 'loops started while the arriving song was heard').toBe(0);
  expect.soft(await looping(), 'a loop playing over the arriving song').toBe(false);
});

// The buffering loop follows a wait for a source (finding, queued; ADR-0041), not an output starting a song it already
// holds: a song started, a paused song resumed and a gapless handover each make none. Before, the loop played for the
// moment of silence at every one of them.
test('the buffering loop sounds while a source is found, and not as the output starts a song it holds', async ({ page, request }) => {
  const wait = L.albums[L.albumIds[141]].trackIds[0];             // a Soulseek song: no peer answers for it here
  const album = L.tracks[L.savedIds[0]].albumId, [a, b] = L.albums[album].trackIds;
  await listen(page);
  await page.route((url) => url.pathname === `/api/stream/${wait}`, () => {});
  await openScreen(page, `/album/${L.tracks[wait].albumId}`, { clock: false });
  await page.evaluate(() => App.needSounds().then(() => App.sounds.load()));
  await page.mouse.click(5, 5);                  // the listener's first tap: sounds may play from here
  await page.evaluate((id) => App.player.playList([id], 0), wait);
  await request.post('/__fixture/emit', { data: { topic: 'stream', data: {
    trackId: wait, jobId: 'wait-' + wait, status: 'finding', attempt: 0, received: 0, size: 0, peersFound: 0, failedPeers: [] } } });
  await page.waitForFunction(() => window.__sounds.some((s) => s.loop), null, { timeout: 15000 });   // a wait for a source still does
  await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
  await page.waitForSelector('#view .trow[data-track-id]');

  const mark = () => page.evaluate(() => window.__sounds.length);
  const loopsSince = (n) => page.evaluate((n) => window.__sounds.slice(n).filter((s) => s.loop).length, n);
  const sounds = async (id) => {
    await page.waitForFunction((id) => App.player.current().id === id && App.player.heard() && App.player.state.pos > 0.3, id, { timeout: 20000 });
    await page.waitForTimeout(800);
  };
  let n = await mark();
  await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
  await sounds(a);
  expect.soft(await loopsSince(n), 'loops started as a saved song started').toBe(0);

  await page.evaluate(() => App.player.pause());
  await page.waitForTimeout(800);
  n = await mark();
  await page.evaluate(() => App.player.play());
  await sounds(a);
  expect.soft(await loopsSince(n), 'loops started as the paused song resumed').toBe(0);

  n = await mark();
  await page.evaluate(() => App.player.seek(App.player.duration() - 2));
  await sounds(b);
  expect.soft(await loopsSince(n), 'loops started at the gapless handover into the next song').toBe(0);
});

// The buffering loop follows a wait for a source wherever the hub reports it (ADR-0041): a found peer whose first bytes
// are still to come (the hub's `buffering`, after `finding`) and a peer the hub falls back to from a stalled one
// (`fallback`) are waits for a source as much as finding one, and the loop plays on through them while nothing sounds.
// `buffering` names the output's own wait too, so the status word alone can't tell them apart.
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
  /* the loop goes on through what the hub says next: not stopped once, still playing a while later */
  async function playsOn(what, next) {
    const n = await loopStops();
    await next();
    await page.waitForTimeout(1500);
    expect.soft(await page.evaluate(() => App.player.heard()), `${what}: nothing sounds`).toBe(false);
    expect.soft(await loopStops() - n, `${what}: times the loop was stopped`).toBe(0);
    expect.soft(await looping(), `${what}: the loop plays`).toBe(true);
  }

  await page.evaluate((id) => App.player.playList([id], 0), found);
  await say(found, 'finding');
  await expect.poll(looping, { message: 'the loop, while a source is found', timeout: 15000 }).toBe(true);
  await playsOn('a found peer\'s first bytes awaited', async () => {
    await say(found, 'buffering', { peer: 'mira' });
    await page.waitForFunction(() => App.player.state.status === 'buffering' && App.player.state.peer && App.player.state.peer.name === 'mira', null, { timeout: 10000 });
  });

  await page.evaluate((id) => App.player.playList([id], 0), fell);
  await say(fell, 'stalled', { peer: 'mira' });
  await page.waitForFunction(() => App.player.state.status === 'stalled', null, { timeout: 10000 });
  await expect.poll(looping, { message: 'the loop, while a stalled peer leaves silence', timeout: 5000 }).toBe(true);
  await playsOn('the hub falls back to another peer', async () => {
    await say(fell, 'fallback', { peer: 'nova', attempt: 1, failedPeers: ['mira'] });
    await page.waitForFunction(() => App.player.state.fallback && App.player.state.status === 'buffering', null, { timeout: 10000 });
  });
});

// The loop's three edges from the round-3 review (ADR-0041). A song that sounded, then fell back to another peer whose
// bytes are still to come, is a wait for a source once its sound stops, however many of the output's `time` events
// came between the hub's words (R1). A song that already sounded, paused early and resumed from what it holds, is the
// output starting a song it holds, whatever the hub said while it was paused (R2); and so is a gapless handover into a
// song the hub was still fetching while it was next (R3).
const soundsAt = (page) => page.evaluate(() => ({ heard: window.__heard.length, sounds: window.__sounds.length }));
const loopsSince = (page, n) => page.evaluate((n) => window.__sounds.slice(n).filter((s) => s.loop).length, n);
const streamSays = (request, id, job) => (status, more = {}) => request.post('/__fixture/emit', { data: { topic: 'stream', data: {
  trackId: id, jobId: job + '-' + id, attempt: 0, received: 0, size: 4000000, peersFound: 3, peer: 'mira', failedPeers: [], status, ...more } } });
/* a song's stream: it comes once `go()` is called (at once with `now`), and never again once `held` is set */
async function holdable(page, id, { now = true } = {}) {
  let go; const s = { held: false, came: new Promise((r) => { go = r; }) };
  s.go = go; if (now) go();
  await page.route((url) => url.pathname === `/api/stream/${id}`, async (route) => { if (s.held) return; await s.came; if (!s.held) await route.continue(); });
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

test('a Soulseek song paused in its first bytes and resumed from what it holds makes no loop, whatever the hub said meanwhile', async ({ page, request }) => {
  const song = L.albums[L.albumIds[142]].trackIds[1];
  await listen(page);
  const stream = await holdable(page, song, { now: false });
  await ready(page, `/album/${L.tracks[song].albumId}`);
  const say = streamSays(request, song, 'early');
  await page.evaluate((id) => App.player.playList([id], 0), song);
  await say('buffering');
  await expect.poll(() => loopsSince(page, 0), { message: 'the loop, while the found peer\'s first bytes come', timeout: 15000 }).toBeGreaterThan(0);
  stream.go();                   // its first bytes: the song sounds
  await sounding(page, song);
  await page.evaluate(() => App.player.pause());
  // the transfer is still within its first 256 KB: the hub says `buffering` while the song is paused
  await say('buffering', { received: 200000 });
  await page.waitForFunction(() => App.player.state.status === 'buffering', null, { timeout: 10000 });
  stream.held = true;            // nothing more comes: the song resumes from what the output holds
  const at = await soundsAt(page);
  await page.evaluate(() => App.player.play());
  await sounding(page, song);
  await page.waitForTimeout(800);
  expect(await loopsSince(page, at.sounds), 'loops started as the paused song resumed').toBe(0);
});

for (const status of ['buffering', 'fallback']) {
  test(`a gapless handover into a song the hub said was ${status} while it was next makes no loop`, async ({ page, request }) => {
    const album = L.tracks[L.savedIds[0]].albumId, a = L.albums[album].trackIds[0];
    const [wait, b] = L.albums[L.albumIds[143]].trackIds.slice(status === 'buffering' ? 0 : 2);
    let asked = 0;
    await listen(page);
    await page.route((url) => url.pathname === `/api/stream/${wait}`, () => {});
    await page.route((url) => url.pathname === `/api/stream/${b}`, (route) => { asked++; route.continue(); });
    await ready(page, `/album/${L.tracks[wait].albumId}`);
    // a wait for a source first: the loop is loaded and plays
    await page.evaluate((id) => App.player.playList([id], 0), wait);
    await streamSays(request, wait, 'wait')('finding');
    await expect.poll(() => loopsSince(page, 0), { message: 'the loop, while a source is found', timeout: 15000 }).toBeGreaterThan(0);
    await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
    await page.waitForSelector('#view .trow[data-track-id]');

    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
    await sounding(page, a);
    await page.evaluate(() => App.player.seek(App.player.duration() - 8));
    await expect.poll(() => asked, { message: 'the next song is made ready', timeout: 10000 }).toBeGreaterThan(0);
    await streamSays(request, b, 'next')(status, { attempt: 1, received: 100000, ...(status === 'fallback' ? { peer: 'nova', failedPeers: ['mira'] } : {}) });
    await page.waitForTimeout(500);
    const at = await soundsAt(page);
    await page.evaluate(() => App.player.seek(App.player.duration() - 1.5));
    await sounding(page, b);
    await page.waitForTimeout(800);
    expect(await loopsSince(page, at.sounds), `loops started at the gapless handover (the hub's word: ${status})`).toBe(0);
  });
}

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
