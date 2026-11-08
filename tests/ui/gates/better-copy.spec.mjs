// A better copy shows the moment the song switches to it: the player bar's quality chip (with its quiet arrow), its
// source, Now Playing and Source details all name the new copy. The owner (2026-11-07): "the source on the bottom left
// didn't update to let me know". The player dropped the new copy's job's events until the crossfade ended, and a copy
// that had all arrived sent no more. And the next song's offer survives a gapless hand-over: its stream opens before it
// plays, and the player kept nothing of what the hub said of it then.
//
// The output is a stand-in for the app's native player (window.LullNative.player, docs/context/web.md#output-interface)
// whose switchTo commits at once: lining copies up by ear is align.js's (tests/web), and the fixture's songs are silent.
// The hub's half is the fixture's stand-in for POST /api/stream/{id}/switch (soulseek's Fetcher::adopt).
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, openScreen } from './lib.mjs';

const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;

/* a native player that plays nothing: time runs while it "plays", ended() hands over to the next item, switchTo commits */
function fakeOutput() {
  const subs = {};
  let cur = null, next = null, pos = 0, playing = false;
  const fire = (type, d) => (subs[type] || []).forEach((fn) => fn(d));
  setInterval(() => {
    if (!cur) return;
    if (playing) pos += 0.1;
    fire('time', { key: cur.key, position: pos, duration: cur.duration || 240, playing, waiting: false });
  }, 100);
  window.__out = {
    switches: [],
    end() { const was = cur; cur = next; next = null; pos = 0; fire('ended', { key: was.key, next: cur ? cur.key : null }); },
  };
  window.LullNative = {
    player: {
      on(type, fn) { (subs[type] = subs[type] || []).push(fn); return () => { subs[type] = subs[type].filter((f) => f !== fn); }; },
      load(item, o) { cur = item; next = null; pos = (o && o.at) || 0; playing = !o || o.autoplay !== false; },
      play() { playing = true; },
      pause() { playing = false; },
      seek(s) { pos = s; },
      setVolume() {},
      setNext(item) { next = item; },
      crossfade() {},
      switchTo(item, o) {
        window.__out.switches.push(item.url);
        return o.commit().then(() => ({ switched: true, residualMs: 0.01, offsetS: 0 }));
      },
    },
  };
}

const youtube = (id, job, size) => ({
  trackId: id, jobId: job, status: 'streaming', attempt: 1, peer: 'YouTube', format: 'YouTube · Opus 160', youtube: true,
  received: size, size, ranges: [[0, 1]], speed: 0, failedPeers: [], peersFound: 0, belowMinimum: false, unverified: false,
  upgrade: { peer: 'quietcanal', path: 'Music\\Artist\\Album\\01 - Song.mp3', format: 'MP3 320', status: 'streaming', received: size, size, ready: true },
});

async function setUp(page, request, n) {
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  await page.addInitScript(fakeOutput);
  await openScreen(page, '/home', { clock: false });
  const ids = await page.evaluate((n) => App.data.api('/__fixture/unsaved-songs?n=' + n).then((r) => { App.data.absorb(r.entities); return r.items; }), n);
  await page.evaluate((list) => App.player.playList(list, 0), ids);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.2);
  const emit = (data) => request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() }, data: { topic: 'stream', data } });
  return { ids, emit };
}

/* every surface that names the copy playing, as it is drawn now */
const surfaces = (page) => page.evaluate(() => {
  const chip = document.querySelector('#pbar .pbar__badges .q-chip');
  return {
    chip: chip ? chip.textContent.trim() : null,
    better: !!chip && chip.classList.contains('q-chip--better'),
    tip: chip ? chip.getAttribute('data-tip') : null,
    marked: document.querySelector('#pbar').getAttribute('data-better-copy'),
    peer: App.player.state.peer && App.player.state.peer.name,
    copy: App.player.playingCopy().format,
  };
});

for (const design of designs) {
  test(`a switch to a better copy shows on every surface at once / ${design}`, async ({ page, request }) => {
    await page.addInitScript((d) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: 'day' })), design);
    const { ids, emit } = await setUp(page, request, 1);
    await emit(youtube(ids[0], 'yt-job', 4000000));
    // the player switches at once (the stand-in commits), the hub answers with the copy's own job
    await expect.poll(() => page.evaluate(() => window.__out.switches.length), { timeout: 15_000 }).toBe(1);
    await expect.poll(() => surfaces(page)).toEqual({ chip: 'MP3 320', better: true, tip: expect.stringMatching(/^Switched to this better copy from YouTube · Opus 160 at \d+:\d\d$/),
      marked: 'true', peer: 'quietcanal', copy: 'MP3 320' });
    // nothing more comes from the hub (the copy had all arrived): the chips stay right, and no toast was raised
    await page.waitForTimeout(1500);
    expect((await surfaces(page)).chip).toBe('MP3 320');
    expect(await page.locator('.toast').count(), 'a quiet mark, no toast').toBe(0);
    // another fetch of the song (another device's, say) is not this play's
    await emit({ ...youtube(ids[0], 'someone-elses', 4000000), peer: 'stranger', format: 'MP3 128', upgrade: null });
    await page.waitForTimeout(500);
    expect((await surfaces(page)).peer).toBe('quietcanal');
    // Source details and Now Playing name it too
    await page.evaluate(() => App.hooks.sourceDetails());
    const sheet = page.locator('.disc-sheet');
    await expect(sheet).toBeVisible();
    await expect(sheet).toContainText('quietcanal');
    await expect(sheet).toContainText('MP3');
    await page.evaluate(() => { App.closeTop(); location.hash = '#/now'; });
    await page.waitForFunction(() => App.current.path === '/now');
    const card = page.locator('.now-srccard');
    await expect(card).toContainText('quietcanal');
    await card.locator('[data-act="now-details"]').click();
    await expect(card).toContainText(/Switched from YouTube · Opus 160 to MP3 320 from quietcanal at \d+:\d\d\./);
  });
}

test('the next song\'s better copy survives a gapless hand-over', async ({ page, request }) => {
  const { ids, emit } = await setUp(page, request, 2);
  // the next song's stream opens before it plays (gapless): the hub says what it is then, and nothing more after
  await page.waitForFunction((id) => App.player.state.queue.findIndex((e) => e.trackId === id) === 1, ids[1]);
  await emit(youtube(ids[1], 'yt-next', 3000000));
  await page.waitForTimeout(300);
  await page.evaluate(() => window.__out.end());
  await page.waitForFunction((id) => App.player.current().id === id, ids[1]);
  await expect.poll(() => page.evaluate(() => window.__out.switches.length), { timeout: 15_000 }).toBe(1);
  await expect.poll(() => surfaces(page)).toMatchObject({ chip: 'MP3 320', better: true, peer: 'quietcanal', marked: 'true' });
});

test('a better copy that can\'t be switched to says why in Activity', async ({ page, request }) => {
  const { ids, emit } = await setUp(page, request, 1);
  // the song is nearly over: the better copy plays from the next play, and the hub is told why
  await emit({ ...youtube(ids[0], 'yt-late', 4000000), upgrade: null });
  await page.waitForFunction(() => App.player.state.received >= 1);
  await page.evaluate(() => App.player.seek(App.player.duration() - 10));
  await page.waitForFunction(() => App.player.state.pos > App.player.duration() - 12);
  await emit(youtube(ids[0], 'yt-late', 4000000));
  await expect.poll(() => request.get('/__fixture/not-switched', { headers: { 'x-fixture-client': fixtureClient() } }).then((r) => r.json()))
    .toEqual([{ trackId: ids[0], peer: 'quietcanal', path: 'Music\\Artist\\Album\\01 - Song.mp3', why: 'nearEnd' }]);
  expect(await page.evaluate(() => window.__out.switches.length)).toBe(0);
});
