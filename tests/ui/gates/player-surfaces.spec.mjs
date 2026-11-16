// One player view for every surface (ADR-0041). The player's state reached the screen along about ten paths, each with
// its own trigger, and the mini player windows repainted from a hand-kept copy of the bar's painter. What that left, as
// found in desktop app 0.2.10: the mini player drew the peer's percentage once and the catalogue's format; four of five
// designs said nothing of "finding / buffering / failed" there; it froze for good once main reloaded, and a mini player
// and a main of different versions talked past each other; a pause the system made went on showing as playing; and
// Home's lead kept the song it was drawn with. Now main derives one view (web/player-view.js), publishes it when it
// changes even while its window is hidden, sends it to the mini player as it is, with a version, and both windows say
// hello. These gates play, skip, pause from the system and fail songs with main hidden (document.hidden,
// visibilityState and the visibilitychange event), and feed the mini player only the views main really sent.
// docs/context/quality.md#ui-gates.
//
// Per design, only what each design draws differently: the mini player's markup (lull, Petal, Skydeck and Starlight
// have a miniPlayer slot of their own; Glasshouse draws the default) and Home's lead slot. The strip is the same default
// markup in every design, so it runs once. The engine's pause, and the link between the windows (hello, bye, the
// version), are the same code whatever the design: once each, in lull.

import { expect, test } from '@playwright/test';
import * as L from '../fixture-hub/library.mjs';
import { designs, fakeOutput, openScreen } from './lib.mjs';
import { fireView, got, openMain, openMini, shown, views } from './player-windows.mjs';

const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;
const LOADING = ['finding', 'queued', 'buffering', 'stalled'];

/* The catalogue's unsaved songs (Soulseek songs: a transfer the fixture can trickle), from albums no other gate streams:
   140 starts /__fixture/unsaved-songs (better-copy, kept-seek, race-seek, like-saves, search-covers), 147 is the feed's
   first new release (fit, sounds, toasts and others), 148 and 149 are source-details' and the broken and failing albums.
   Only songs whose titles neither contain nor are in another's, so a title found on a surface is that song's.
   The fixture hub keeps one transfer per song for every page: a second trickle of a song restarts the first, and a page
   that streams a song another page trickles gets that transfer. So every test trickles songs of its own, one per test and
   repeat (the gate runs fullyParallel), and the songs played as they are, which any test may share, are never trickled. */
const pool = [];
for (const album of L.albumIds.slice(141, 147)) {
  for (const id of L.albums[album].trackIds) {
    const t = L.tracks[id];
    if (!pool.some((k) => k.title.includes(t.title) || t.title.includes(k.title))) pool.push(t);
  }
}
const [plain1, plain2, copied, afterCopied] = pool;   // played as they are, never trickled
const OWN = 12;   // trickled per repeat: the mini player's and the skip's per design, the strip's and Connect's
const REPEATS = Math.floor((pool.length - 4) / OWN);
/* the song this test trickles: `slot` (0 to OWN - 1) of this repeat's */
function ownSong(slot) {
  const r = test.info().repeatEachIndex;
  if (r >= REPEATS) throw new Error(`player-surfaces has songs of its own for ${REPEATS} repeats: --repeat-each ${REPEATS} at most`);
  return pool[4 + r * OWN + slot];
}
/* what Home's lead shows of the queue, per design (its homeLead slot): lull's desk widgets show neither */
const LEAD = { lull: [], petal: ['next'], skydeck: ['title', 'next'], starlight: ['next'], glasshouse: ['title', 'next'] };

/* one page's own events on the fixture hub; an album's page, so the app knows its songs */
async function mainAt(page, design, route) {
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  return openMain(page, { design, route });
}
/* main on the page of songs[0]'s album, after the pages of the other songs' albums: the app knows every song and has
   asked the hub for their covers, as for songs it has shown. A cover asked for later repaints the whole page (app.js
   repaintCovers), and a Home lead drawn again for that says nothing of the lead. */
async function mainWith(page, design, songs) {
  const albums = [...new Set(songs.map((t) => t.albumId))].reverse();
  const asked = () => page.waitForResponse((r) => r.url().endsWith('/api/catalog/resolve'));
  let covers = asked();
  const relay = await mainAt(page, design, `/album/${albums[0]}`);
  await covers;
  for (const album of albums.slice(1)) {
    covers = asked();
    await page.evaluate((a) => { location.hash = '#/album/' + a; }, album);
    await covers;
  }
  return relay;
}
const trickle = (page, id, bytesPerS) => page.request.post('/__fixture/trickle', { data: { id, bytesPerS } });
const playing = (page) => page.waitForFunction(() => App.player.state.playing && /^(streaming|library)$/.test(App.player.state.status)
  && App.player.state.pos > 0.5, null, { timeout: 20_000 });
/* hides main as the app's window is hidden; when it did, for what must have come after */
const hide = async (page) => { await page.evaluate(() => window.__hide(true)); return Date.now(); };
const mainVersion = (page) => page.evaluate(() => (window.PlayerView ? window.PlayerView.VERSION : null));
/* the share of the file that has arrived, as a view says it (the snapshot before ADR-0041 said it at the top), in whole
   percent: rounded or cut, as a surface may show it */
const shareOf = (v) => ((v.transfer ? v.transfer.received : v.received) || 0) * 100;
const receivedOf = (v) => Math.round(shareOf(v));
const showsShare = (percents, v) => percents.some((p) => p === Math.round(shareOf(v)) || p === Math.floor(shareOf(v)));
const rateOf = (v) => (v.clock ? v.clock.rate : v.playing ? 1 : 0);

/* the mini player's words for the player's state are the bar's, while main's player is in `state` */
async function sameWords(page, mini, relay, inState) {
  await expect.soft.poll(async () => {
    const main = await page.evaluate(() => ({ status: App.player.state.status, playing: App.player.state.playing,
      words: document.querySelector('#pbar').getAttribute('data-status-text') }));
    if (!inState(main)) return `main is ${main.status}`;
    const m = await mini.evaluate(shown);
    return m.status === main.words && main.words ? 'same' : `mini: ${JSON.stringify(m.status)}, bar: ${JSON.stringify(main.words)}`;
  }, { message: 'the mini player\'s [data-dk-status] says what the bar says', timeout: 15_000 }).toBe('same');
  const last = views(relay).at(-1);
  expect.soft(last && last.words, 'the view main sent last carries the same words').toBe(
    await page.evaluate(() => document.querySelector('#pbar').getAttribute('data-status-text')));
}
async function failAt(page) {
  await page.evaluate((a) => { location.hash = '#/album/' + a; }, L.failingAlbum);
  const id = await page.waitForSelector('#view .trow[data-track-id]').then((r) => r.getAttribute('data-track-id'));
  await page.evaluate((x) => App.player.playList([x], 0), id);
}

test.describe('with main hidden', () => {
  designs.forEach((design, i) => {
    test(`the mini player runs on main's views: its time, the transfer and the copy that plays / ${design}`, async ({ page }) => {
      const songs = [ownSong(i), plain1, plain2];
      const relay = await mainWith(page, design, songs);
      await trickle(page, songs[0].id, 24_000);   // three times the song's rate: it plays while the share arrived climbs
      const mini = await openMini(relay, { design });
      await page.evaluate((ids) => App.player.playList(ids, 0), songs.slice(0, 3).map((t) => t.id));
      await playing(page);
      const hidden = await hide(page);
      await page.waitForTimeout(1500);

      // main published views while hidden (the share arriving changes them), not a snapshot of its own: the wire version
      // and a clock
      const v = await mainVersion(page), sent = relay.main.filter((e) => e.name === 'player' && e.at >= hidden).map((e) => e.payload);
      expect.soft(v, 'main has PlayerView.VERSION').not.toBeNull();
      expect.soft(sent.length, 'views sent to the mini player while main was hidden').toBeGreaterThan(0);
      expect.soft(sent.filter((x) => x.v !== v || !x.clock || typeof x.clock.at !== 'number').length,
        'views without the wire version or a clock').toBe(0);

      // the time moves on main's clock with no new view
      relay.toMini = false; await relay.sent;
      const t0 = (await mini.evaluate(shown)).time;
      await page.waitForTimeout(3200);
      const t1 = (await mini.evaluate(shown)).time;
      expect.soft(t1 - t0, 'seconds the mini player moved on in 3.2 s with no view').toBeGreaterThanOrEqual(2);
      expect.soft(t1 - t0).toBeLessThanOrEqual(4);
      relay.toMini = true;

      // the share arrived follows the views, not the first one: views main sent while hidden bring it on
      const before = await got(mini).then((g) => g.filter((e) => e.at < hidden).map((e) => receivedOf(e.payload)));
      await expect.poll(async () => (await got(mini)).some((e) => e.at >= hidden && !before.includes(receivedOf(e.payload))),
        { message: 'a view with a new share reached the mini player while main was hidden', timeout: 10_000 }).toBe(true);
      relay.toMini = false; await relay.sent;   // what it shows, against the last view it got
      const seen = await got(mini), share = { first: seen[0].payload, last: seen.at(-1).payload };
      const m = await mini.evaluate(shown);
      relay.toMini = true;
      expect.soft(receivedOf(share.first), 'the transfer moved on').not.toBe(receivedOf(share.last));
      expect.soft(showsShare(m.received == null ? [] : [parseInt(m.received, 10)], share.last),
        `the mini player's [data-bind="received"] shows the last view's share, ${receivedOf(share.last)}%: it shows ${m.received}`).toBe(true);

      // the format is the copy that plays (the peer's), as the bar has it, not the catalogue's
      const bar = await page.evaluate(() => { const c = document.querySelector('#pbar .q-chip'); return c ? c.textContent.trim() : null; });
      expect(bar, 'the bar shows the peer\'s copy').toBeTruthy();
      expect(bar, 'the peer\'s copy differs from the catalogue\'s').not.toBe(songs[0].format);
      expect.soft(m.chip, 'the mini player\'s format chip').toBe(bar);

      // a pause stops it
      await page.evaluate(() => App.player.pause());
      await expect.poll(async () => { const g = await got(mini); return g.length ? rateOf(g.at(-1).payload) : null; },
        { message: 'a paused view reached the mini player' }).toBe(0);
      const p0 = (await mini.evaluate(shown)).time;
      await page.waitForTimeout(2200);
      expect.soft((await mini.evaluate(shown)).time, 'the time after a pause').toBe(p0);
    });

    test(`a skip, a song that loads and one that fails show on the bar, Home's lead and the mini player / ${design}`, async ({ page }) => {
      const songs = [plain1, plain2, ownSong(designs.length + i)];
      const relay = await mainWith(page, design, songs);
      await trickle(page, songs[2].id, 200);   // far slower than the song plays: it waits for data
      const mini = await openMini(relay, { design });
      await page.evaluate((ids) => App.player.playList(ids, 0), songs.map((t) => t.id));
      await playing(page);
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForSelector('.disc-lead', { state: 'attached' });   // drawn, whether or not the design shows it at this width
      if (LEAD[design].includes('next')) await expect(page.locator('.disc-lead'), 'the lead as drawn').toContainText(`next: ${songs[1].title}`);
      await hide(page);

      await page.evaluate(() => App.player.next());
      const title = songs[1].title, next = songs[2].title;
      await expect.poll(() => page.evaluate(() => document.querySelector('#pbar .pbar__title').textContent.trim()),
        { message: 'the bar\'s title' }).toBe(title);
      await expect.soft.poll(async () => (await mini.evaluate(shown)).title, { message: 'the mini player\'s title' }).toBe(title);
      // Home's lead, drawn while the first song played: the song now playing and the next, not the ones it was drawn with
      // (every lead that names the next song writes "next: <title>"; one that names the song lists the queue from it)
      const lead = () => page.evaluate(() => document.querySelector('.disc-lead').textContent.replace(/\s+/g, ' '));
      if (LEAD[design].includes('next')) {
        await expect.soft.poll(lead, { message: 'Home\'s lead names the next song' }).toContain(`next: ${next}`);
        expect.soft(await lead(), 'Home\'s lead no longer names the song now playing as the next').not.toContain(`next: ${title}`);
      }
      if (LEAD[design].includes('title')) {
        await expect.soft.poll(lead, { message: 'Home\'s lead names the song now playing' }).toContain(title);
        expect.soft(await lead(), 'Home\'s lead no longer names the song before the skip').not.toContain(songs[0].title);
      }

      // the next song waits for its data: loading, in the mini player's words as in the bar's
      await page.evaluate(() => App.player.next());
      await sameWords(page, mini, relay, (s) => s.playing && LOADING.includes(s.status));
      // a song no peer shares: failed, with the hub's reason
      await failAt(page);
      await sameWords(page, mini, relay, (s) => s.status === 'failed');
    });
  });

  // the strip: the time as --song-pos (it has no times of its own), and the player's words in it too
  test('the strip runs on main\'s views and says what a loading or failed song is doing', async ({ page }) => {
    const song = plain1, slow = ownSong(2 * designs.length);
    const relay = await mainWith(page, 'lull', [song, slow]);
    await trickle(page, slow.id, 200);
    const mini = await openMini(relay, { design: 'lull', style: 'strip' });
    await page.evaluate((id) => App.player.playList([id], 0), song.id);
    await playing(page);
    await hide(page);
    await page.waitForTimeout(1000);

    const v = await mainVersion(page);
    expect.soft(v, 'main has PlayerView.VERSION').not.toBeNull();
    expect.soft(views(relay).filter((x) => x.v !== v || !x.clock).length, 'views without the wire version or a clock').toBe(0);
    // no song is longer than 420 s, so a hundredth of it is at most 4.2 s: 5 s moves it on at least one
    relay.toMini = false; await relay.sent;
    const s0 = (await mini.evaluate(shown)).songPos;
    await page.waitForTimeout(5000);
    expect.soft((await mini.evaluate(shown)).songPos, '--song-pos 5 s on, with no view').toBeGreaterThan(s0);
    relay.toMini = true;
    await page.evaluate(() => App.player.pause());
    await expect.poll(async () => { const g = await got(mini); return g.length ? rateOf(g.at(-1).payload) : null; },
      { message: 'a paused view reached the strip' }).toBe(0);
    const p0 = (await mini.evaluate(shown)).songPos;
    await page.waitForTimeout(4500);
    expect.soft((await mini.evaluate(shown)).songPos, '--song-pos after a pause').toBe(p0);

    await page.evaluate((id) => App.player.playList([id], 0), slow.id);
    await sameWords(page, mini, relay, (s) => s.playing && LOADING.includes(s.status));
    await failAt(page);
    await sameWords(page, mini, relay, (s) => s.status === 'failed');
  });
});

// The listener's intent follows the output's own word (ADR-0041): a pause or a play the system made (another app takes
// the audio, headphones out, a headset's keys) is the output's `system` event, and the bar shows it. It is never guessed
// from a `time` with playing false: both outputs report one at a song's end (the browser's element pauses there; the
// fake native output ends as Android's Engine.kt does, lib.mjs fakeOutput), and a guessed pause nothing undid stopped
// repeat-one, a song ending into the next and a crossfade. Each where both outputs differ: the browser's <audio> decks
// and the app's native player.
const outputs = ['browser', 'native'];
const album = L.tracks[L.savedIds[0]].albumId, saved = L.albums[album].trackIds;
async function outputAt(page, output) {
  if (output === 'native') {
    await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
    await page.addInitScript(fakeOutput);
    await openScreen(page, `/album/${album}`, { clock: false });
  } else await mainAt(page, 'lull', `/album/${album}`);
}
/* every change of the player from here on, with the listener's intent as it stood */
const watchIntent = (page) => page.evaluate(() => { window.__intents = []; App.on('player', () => window.__intents.push(App.player.state.playing)); });
const intentDropped = (page) => page.evaluate(() => window.__intents.filter((p) => !p).length);
const playKey = (page) => page.evaluate(() => { const b = document.querySelector('#pbar [data-bind="play"]');
  return { label: b.getAttribute('aria-label'), loading: b.classList.contains('is-buffering') }; });
/* the song to `left` seconds from its end, as the output would get there */
const nearEnd = (page, output, left) => page.evaluate(([native, left]) => {
  const at = App.player.duration() - left;
  if (native) window.__out.at(at); else App.player.seek(at);
}, [output === 'native', left]);
/* the deck the browser's engine plays, paused or played as the system does it */
const deck = (page, act) => page.evaluate((act) => {
  const el = [...window.__played].find((e) => /\/api\/(library\/)?stream\//.test(e.src) && (act === 'play' ? e.paused : !e.paused));
  if (el) { el[act](); if (act === 'pause') window.__systemAt = window.__pauseCalls.length; }
  return !!el;
}, act);
/* what paused the element the system paused, after it did (window.__systemAt, set as it paused it). The engine only
   records a system pause (ADR-0041): answering it with the output's pause halts the deck it just heard from, and tells
   the platform the listener paused (iOS then doesn't resume after a call) */
const pausedAgain = (page) => page.evaluate(() => {
  const calls = window.__pauseCalls, at = window.__systemAt;
  return calls.slice(at).filter((e) => e === calls[at - 1]).length;
});

test.describe('the listener\'s intent', () => {
  for (const output of outputs) {
    test(`stays on as repeat one plays the song again / ${output}`, async ({ page }) => {
      await outputAt(page, output);
      await page.evaluate((id) => App.player.playList([id], 0), saved[0]);
      await playing(page);
      await page.evaluate(() => { while (App.player.state.repeat !== 'one') App.player.cycleRepeat(); });
      await watchIntent(page);
      await nearEnd(page, output, 1.5);
      await page.waitForFunction(() => App.player.state.pos > App.player.duration() - 3);
      await page.waitForFunction(() => App.player.state.pos > 0.3 && App.player.state.pos < 5, null, { timeout: 15_000 });
      expect(await page.evaluate(() => App.player.current().id), 'the same song').toBe(saved[0]);
      expect.soft(await page.evaluate(() => App.player.state.playing), 'the intent').toBe(true);
      expect.soft(await intentDropped(page), 'changes with the intent off').toBe(0);
      expect.soft((await playKey(page)).label, 'the play key').toBe('Pause');
    });

    test(`stays on as a song ends into the next / ${output}`, async ({ page }) => {
      await outputAt(page, output);
      await page.evaluate((ids) => App.player.playList(ids, 0), saved.slice(0, 2));
      await playing(page);
      await watchIntent(page);
      await nearEnd(page, output, 1.5);
      await page.waitForFunction((id) => App.player.current().id === id && App.player.state.pos > 0.5, saved[1], { timeout: 15_000 });
      expect.soft(await page.evaluate(() => App.player.state.playing), 'the intent').toBe(true);
      expect.soft(await intentDropped(page), 'changes with the intent off').toBe(0);
      expect.soft((await playKey(page)).label, 'the play key').toBe('Pause');
    });

    test(`follows a pause and a play the system makes / ${output}`, async ({ page }) => {
      await outputAt(page, output);
      await page.evaluate((id) => App.player.playList([id], 0), saved[2]);
      await playing(page);
      const pauses = output === 'native' ? await page.evaluate(() => window.__out.pauses) : 0;
      if (output === 'native') await page.evaluate(() => window.__out.system(false));
      else expect(await deck(page, 'pause'), 'found the deck the engine plays').toBe(true);
      await expect.soft.poll(() => page.evaluate(() => App.player.state.playing), { message: 'the intent after the system paused' }).toBe(false);
      await expect.soft.poll(async () => (await playKey(page)).label, { message: 'the play key' }).toBe('Play');
      expect.soft(output === 'native' ? await page.evaluate((n) => window.__out.pauses - n, pauses) : await pausedAgain(page),
        'the engine paused the output in answer').toBe(0);
      if (output === 'native') await page.evaluate(() => window.__out.system(true));
      else expect(await deck(page, 'play'), 'found the paused deck').toBe(true);
      await expect.soft.poll(() => page.evaluate(() => App.player.state.playing), { message: 'the intent after the system played' }).toBe(true);
      await expect.soft.poll(async () => (await playKey(page)).label, { message: 'the play key' }).toBe('Pause');
    });

    // a saved song resumed: no status changes, so only the view says the sound came back (ADR-0041: parts showing the
    // activity repaint on view)
    test(`a play key pressed again stops showing loading once the song sounds / ${output}`, async ({ page }) => {
      await outputAt(page, output);
      await page.evaluate((id) => App.player.playList([id], 0), saved[3]);
      await playing(page);
      await page.evaluate(() => App.player.pause());
      await page.waitForTimeout(400);
      await page.evaluate(() => App.player.play());
      await page.waitForFunction(() => App.player.state.pos > 0 && App.player.heard(), null, { timeout: 10_000 });
      await expect.poll(() => playKey(page), { message: 'the play key once the song sounds' }).toEqual({ label: 'Pause', loading: false });
      await page.waitForTimeout(1000);
      expect(await playKey(page), 'and a second on').toEqual({ label: 'Pause', loading: false });
    });
  }

  test('stays on through a crossfade into the next song / browser', async ({ page }) => {
    await outputAt(page, 'browser');
    await page.evaluate(() => App.setPlayback({ crossfade: 3 }));
    await page.evaluate((ids) => App.player.playList(ids, 0), saved.slice(4, 6));
    await playing(page);
    await page.evaluate(() => App.setPlayback({ crossfade: 3 }));   // after the hub's settings arrived, too
    await watchIntent(page);
    await page.evaluate(() => App.on('player', () => { if (App.player.state.crossfade) window.__faded = true; }));
    await nearEnd(page, 'browser', 6);
    await page.waitForFunction((id) => App.player.current().id === id && App.player.state.pos > 0.5, saved[5], { timeout: 20_000 });
    expect.soft(await page.evaluate(() => !!window.__faded), 'the songs crossfaded').toBe(true);
    expect.soft(await page.evaluate(() => App.player.state.playing), 'the intent').toBe(true);
    expect.soft(await intentDropped(page), 'changes with the intent off').toBe(0);
    expect.soft((await playKey(page)).label, 'the play key').toBe('Pause');
  });

  // Connect's `playing` is the intent (ADR-0041): a device taking over plays on what the listener wanted, loading or not
  test('is what Connect reports for a song still loading', async ({ page }) => {
    const song = ownSong(2 * designs.length + 1);
    await mainWith(page, 'lull', [song]);
    const reports = [];
    page.on('request', (r) => { if (r.method() === 'POST' && r.url().endsWith('/api/devices/me/state')) reports.push(r.postDataJSON()); });
    await trickle(page, song.id, 200);
    await page.evaluate((id) => App.player.playList([id], 0), song.id);
    await page.waitForFunction(() => App.player.state.playing && /^(finding|queued|buffering|stalled)$/.test(App.player.state.status), null, { timeout: 15_000 });
    await page.waitForTimeout(1500);
    const mine = reports.filter((b) => b.trackId === song.id);
    expect(mine.length, 'reports of the song').toBeGreaterThan(0);
    expect(mine.at(-1).playing, 'the last report\'s playing').toBe(true);
  });
});

// A system pause reaches the intent on every element the browser's output plays, however it got there (ADR-0041): the
// output keeps per element whether it wants it playing. Counting the pauses and plays it asked for leaked, because
// loading an element drops the events it had queued: a deck the output paused and loaded (the old song's, after a
// skip with the next preloaded, a crossfade or a better-copy switch) kept a pause it never heard back, and swallowed
// the listener's next real one once that deck played again. Each case below plays on until the reused deck is the
// playing one, then pauses that element directly, as the system does.
const active = () => [...window.__played].find((e) => !e.paused && /\/api\/(library\/)?stream\//.test(e.src));
/* the playing element, marked: the deck that gets reused */
const markFirst = (page) => page.evaluate((f) => { const el = eval(f)(); if (el) el.__mark = 'first'; return !!el; }, `(${active})`);
const firstHolds = (page, id) => page.waitForFunction((id) => window.__audios.some((e) => e.__mark === 'first' && e.src.includes(id)), id, { timeout: 15_000 });
const firstPlays = (page, id) => page.waitForFunction(([f, id]) => { const el = eval(f)();
  return el && el.__mark === 'first' && el.src.includes(id) && App.player.current().id === id && App.player.state.pos > 0.3; }, [`(${active})`, id], { timeout: 20_000 });
const nowPlays = (page, id) => page.waitForFunction((id) => App.player.current().id === id && App.player.heard() && App.player.state.pos > 0.3, id, { timeout: 20_000 });
async function systemPauseOnFirst(page) {
  expect(await page.evaluate(() => {
    const el = [...window.__played].find((e) => e.__mark === 'first' && !e.paused);
    if (el) { el.pause(); window.__systemAt = window.__pauseCalls.length; }
    return !!el;
  }), 'the reused deck plays').toBe(true);
  await expect.soft.poll(() => page.evaluate(() => App.player.state.playing), { message: 'the intent after the system paused the reused deck' }).toBe(false);
  await expect.soft.poll(async () => (await playKey(page)).label, { message: 'the play key' }).toBe('Play');
  expect.soft(await pausedAgain(page), 'the engine paused the output in answer').toBe(0);
}

test.describe('a system pause on a reused deck', () => {
  test('after a skip with the next song preloaded', async ({ page }) => {
    const [a, b, c] = saved.slice(6, 9);
    await outputAt(page, 'browser');
    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b, c]);
    await playing(page);
    expect(await markFirst(page)).toBe(true);
    await nearEnd(page, 'browser', 80);   // within the preload's lead: the next song loads on the other deck
    await page.waitForFunction((id) => window.__audios.some((e) => e.__mark !== 'first' && e.src.includes(id)), b, { timeout: 15_000 });
    await page.evaluate(() => App.player.next());
    await nowPlays(page, b);
    await nearEnd(page, 'browser', 80);
    await firstHolds(page, c);            // the old song's deck, paused and loaded, now holds the next one
    await page.evaluate(() => App.player.next());
    await firstPlays(page, c);
    await systemPauseOnFirst(page);
  });

  test('after a crossfade that finished', async ({ page }) => {
    const [a, b, c] = saved.slice(9, 12);
    await outputAt(page, 'browser');
    await page.evaluate(() => App.setPlayback({ crossfade: 3 }));
    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b, c]);
    await playing(page);
    await page.evaluate(() => App.setPlayback({ crossfade: 3 }));
    expect(await markFirst(page)).toBe(true);
    await nearEnd(page, 'browser', 6);
    // the fade runs on the wall clock for as long as the song had left; the outgoing deck lags it (half speed, as a deck
    // behind its clock is), so the fade finishes while that deck still plays and the output pauses and lets it go
    await page.waitForFunction(() => App.player.state.crossfade, null, { timeout: 10_000 });
    await page.evaluate(() => { [...window.__played].find((e) => e.__mark === 'first').playbackRate = 0.5; });
    await nowPlays(page, b);
    await page.evaluate(() => { [...window.__played].find((e) => e.__mark === 'first').playbackRate = 1; });
    await nearEnd(page, 'browser', 6);
    await firstPlays(page, c);
    await systemPauseOnFirst(page);
  });

  test('after a better-copy switch that finished', async ({ page, request }) => {
    const [a, b] = [copied, afterCopied].map((t) => t.id);
    await mainWith(page, 'lull', [copied, afterCopied]);
    await request.post('/__fixture/noise', { data: { id: a } });   // sound to line the copies up by
    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
    await playing(page);
    expect(await markFirst(page)).toBe(true);
    const size = 3_000_000;
    await request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() }, data: { topic: 'stream', data: {
      trackId: a, jobId: 'noise-job', status: 'streaming', attempt: 1, peer: 'slowpeer', format: 'MP3 128', youtube: false,
      received: size, size, ranges: [[0, 1]], speed: 0, failedPeers: [], peersFound: 1, belowMinimum: false, unverified: false,
      upgrade: { peer: 'quietcanal', path: 'Music\\Artist\\Album\\02 - Song.flac', format: 'FLAC 16/44.1', status: 'streaming', received: size, size, ready: true } } } });
    await expect.poll(() => page.evaluate(() => { const u = App.player.state.upgrade; return u ? u.state + (u.note ? ': ' + u.note : '') : null; }),
      { message: 'the better copy switched', timeout: 45_000 }).toBe('switched');
    await nearEnd(page, 'browser', 80);
    await firstHolds(page, b);            // the first copy's deck, let go at the switch, now holds the next song
    await page.evaluate(() => App.player.next());
    await firstPlays(page, b);
    await systemPauseOnFirst(page);
  });
});

// An output that reports a system pause has already stopped what it was doing (ADR-0041): a crossfade in progress is
// abandoned, not finished while silent. The next song isn't handed over, nothing sounds from its deck, and the song
// isn't recorded as completed. The engine only records the pause; it doesn't pause the output in answer. A play the
// system then makes on the abandoned fade's incoming deck, which the output isn't playing, is halted, not followed.
test.describe('a system pause during a crossfade', () => {
  test('abandons the fade / browser', async ({ page }) => {
    const [a, b] = saved.slice(12, 14);
    await outputAt(page, 'browser');
    const ends = [];
    page.on('request', (r) => { if (r.method() === 'PATCH' && /\/api\/plays\//.test(r.url())) ends.push(r.postDataJSON()); });
    await page.evaluate(() => App.setPlayback({ crossfade: 5 }));
    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
    await playing(page);
    await page.evaluate(() => App.setPlayback({ crossfade: 5 }));
    await nearEnd(page, 'browser', 6);
    await page.waitForFunction(() => App.player.state.crossfade, null, { timeout: 10_000 });
    const before = ends.length;
    expect(await page.evaluate((a) => {
      const el = [...window.__played].find((e) => !e.paused && e.src.includes(a));
      if (el) { el.pause(); window.__systemAt = window.__pauseCalls.length; }
      return !!el;
    }, a), 'the fading song\'s deck').toBe(true);
    await page.waitForTimeout(7000);      // past where the fade would have ended
    const r = await page.evaluate((b) => ({ id: App.player.current().id, index: App.player.state.index, playing: App.player.state.playing,
      crossfade: App.player.state.crossfade, incoming: [...window.__played].filter((e) => e.src.includes(b)).map((e) => e.paused) }), b);
    expect.soft(r.id, 'the song is still the fading one').toBe(a);
    expect.soft(r.index, 'the queue did not move on').toBe(0);
    expect.soft(r.playing, 'the intent').toBe(false);
    expect.soft(r.crossfade, 'the fade').toBe(false);
    expect.soft(r.incoming.every((p) => p), 'the next song\'s deck is silent').toBe(true);
    expect.soft(ends.slice(before).filter((e) => e.completed).length, 'plays recorded as completed').toBe(0);
    expect.soft(await pausedAgain(page), 'the engine paused the output in answer').toBe(0);

    expect(await page.evaluate((b) => { const els = [...window.__played].filter((e) => e.src.includes(b) && e.paused); els.forEach((e) => e.play()); return els.length; }, b),
      'the incoming deck, played by the system').toBeGreaterThan(0);
    await expect.soft.poll(() => page.evaluate((b) => [...window.__played].filter((e) => e.src.includes(b)).every((e) => e.paused), b),
      { message: 'the incoming deck, halted' }).toBe(true);
    expect.soft(await page.evaluate(() => App.player.state.playing), 'the intent after the system played the incoming deck').toBe(false);
    expect.soft(await page.evaluate(() => App.player.current().id), 'the song after the system played the incoming deck').toBe(a);
  });

  test('abandons the fade / native', async ({ page }) => {
    const [a, b] = saved.slice(12, 14);
    await outputAt(page, 'native');
    await page.evaluate(() => App.setPlayback({ crossfade: 5 }));
    await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
    await playing(page);
    await page.evaluate(() => App.setPlayback({ crossfade: 5 }));
    await nearEnd(page, 'native', 6);
    await page.waitForFunction(() => App.player.state.crossfade, null, { timeout: 10_000 });
    const pauses = await page.evaluate(() => window.__out.pauses);
    await page.evaluate(() => window.__out.system(false));   // the output abandons its fade, then says so
    await expect.soft.poll(() => page.evaluate(() => App.player.state.crossfade), { message: 'the fade, abandoned' }).toBe(false);
    expect.soft(await page.evaluate((n) => window.__out.pauses - n, pauses), 'the engine paused the output in answer').toBe(0);
    expect.soft(await page.evaluate(() => App.player.state.playing), 'the intent').toBe(false);
    expect.soft(await page.evaluate(() => App.player.current().id), 'the song is still the fading one').toBe(a);
  });
});

// What the view says has arrived to play is what the output holds (ADR-0041: transfer.buffered is the output's ranges,
// cut to what arrived), not the transfer: a saved song has all arrived, while the output may hold only its first seconds.
test('the view\'s buffered share is what the output holds, not what arrived / native', async ({ page }) => {
  const a = saved[3];
  await outputAt(page, 'native');
  await page.evaluate((id) => App.player.playList([id], 0), a);
  await playing(page);
  const dur = await page.evaluate(() => App.player.duration());
  await page.evaluate((s) => window.__out.buffered([[0, s]]), dur / 4);
  await expect.poll(() => page.evaluate(() => App.player.view().transfer.buffered), { message: 'the view\'s transfer.buffered' }).toBeCloseTo(0.25, 2);
  expect(await page.evaluate(() => App.player.view().transfer.received), 'what arrived: all of it').toBe(1);
});

// A hidden main still publishes its view (ADR-0041): a change of what the output holds changes nothing else, so only the
// player's clock publishes it, and that clock must not skip the page while nobody sees it.
test('a hidden main publishes what the output holds / native', async ({ page }) => {
  const a = saved[5];
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  await page.addInitScript(fakeOutput);
  await openMain(page, { route: `/album/${album}` });
  await page.evaluate(() => { window.__seen = []; App.on('view', (v) => window.__seen.push(v)); });
  await page.evaluate((id) => App.player.playList([id], 0), a);
  await playing(page);
  const dur = await page.evaluate(() => App.player.duration());
  await hide(page);
  await page.evaluate((s) => window.__out.buffered([[0, s]]), dur / 3);
  await expect.poll(() => page.evaluate(() => window.__seen.some((v) => Math.abs(v.transfer.buffered - 1 / 3) < 0.01)),
    { message: 'a view with the output\'s third, published while main was hidden', timeout: 3000 }).toBe(true);
});

// Connect hears every change of what the player is doing, not only a play or a pause (ADR-0041): a song that fails while
// it plays clears the intent, and the other devices learn it at once, not at the next heartbeat.
test('Connect reports a song that fails while it plays / native', async ({ page }) => {
  const a = saved[4];
  await outputAt(page, 'native');
  const reports = [];
  page.on('request', (r) => { if (r.method() === 'POST' && r.url().endsWith('/api/devices/me/state')) reports.push({ at: Date.now(), ...r.postDataJSON() }); });
  await page.evaluate((id) => App.player.playList([id], 0), a);
  await playing(page);
  await expect.poll(() => reports.some((b) => b.trackId === a && b.playing), { message: 'a report of the song playing' }).toBe(true);
  await page.waitForTimeout(1500);      // past the second a report may wait, so what comes next answers the failure
  const failedAt = Date.now();
  await page.evaluate(() => window.__out.fail('This file can\'t be played here'));
  await expect.poll(() => page.evaluate(() => App.player.view().activity), { message: 'the song failed' }).toBe('error');
  await expect.poll(() => reports.some((b) => b.at >= failedAt && b.trackId === a && b.playing === false),
    { message: 'a report of the song, not playing, within 3 s of it failing', timeout: 3000 }).toBe(true);
});

// A system pause applies whatever song it names: the intent is the player's, not a song's. A system play applies only
// to the song that plays (ADR-0041). Here the song a pause or play names has already ended into the next.
test('a system pause naming the song before stops the intent; a play naming it does not / native', async ({ page }) => {
  const [a, b] = saved.slice(0, 2);
  await outputAt(page, 'native');
  await page.evaluate((ids) => App.player.playList(ids, 0), [a, b]);
  await playing(page);
  const before = await page.evaluate(() => window.__out.keys[0]);
  await nearEnd(page, 'native', 1.5);
  await page.waitForFunction((id) => App.player.current().id === id && App.player.state.pos > 0.5, b, { timeout: 15_000 });
  // the fake's events reach the engine at once: each step's intent is read straight after it
  const intentAfter = (p, key) => page.evaluate(([p, key]) => { window.__out.system(p, key || undefined); return App.player.state.playing; }, [p, key]);
  expect.soft(await intentAfter(false, before), 'the intent after a system pause naming the song before').toBe(false);
  expect(await intentAfter(true, null), 'the intent after a system play of the song that plays').toBe(true);
  expect(await intentAfter(false, null), 'the intent after a system pause of the song that plays').toBe(false);
  expect.soft(await intentAfter(true, before), 'the intent after a system play naming the song before').toBe(false);
});

/* main playing a saved song, and a mini player that has its views */
async function linked(page) {
  const song = L.savedIds[1];
  const relay = await mainAt(page, 'lull', `/album/${L.tracks[song].albumId}`);
  const mini = await openMini(relay, { design: 'lull' });
  await page.evaluate((id) => App.player.playList([id], 0), song);
  await playing(page);
  await expect.poll(async () => { const g = await got(mini); return g.length ? rateOf(g.at(-1).payload) : null; },
    { message: 'a playing view reached the mini player' }).toBe(1);
  return { relay, mini };
}
const hellos = (relay) => relay.mini.filter((e) => e.name === 'mini' && e.payload && e.payload.action === 'hello');

// main reloads (a crash, the hub-unreachable page, a reconnect) while the mini player stays open: main says hello as its
// page starts, the mini player answers with hello, and main sends it views again instead of leaving it frozen
test('a reloaded main learns the mini player is open and sends it views again', async ({ page }) => {
  const { relay, mini } = await linked(page);
  const before = hellos(relay).length, reloaded = Date.now();
  await page.reload();
  await page.waitForFunction(() => window.App && App.player);
  await expect.soft.poll(() => relay.main.some((e) => e.name === 'main-hello' && e.at >= reloaded), { message: 'main said hello' }).toBe(true);
  await expect.poll(() => hellos(relay).length, { message: 'the mini player said hello again' }).toBeGreaterThan(before);
  const v = await mini.evaluate(shown).then((s) => s.version);
  expect.soft(hellos(relay).at(-1).payload.v, 'the hello carries the mini player\'s version').toBe(v);
  expect.soft(v, 'the mini player has PlayerView.VERSION').not.toBeNull();
  await expect.poll(async () => (await got(mini)).some((e) => e.at >= reloaded), { message: 'views reached the mini player after the reload' }).toBe(true);
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'the link' }).toBe('ok');
});

// main goes (its page hides, or its renderer dies and the app says so): the mini player stops its clock and says lull's
// window is away, until a view comes
test('when main says bye the mini player stops and says main is away until a view comes', async ({ page }) => {
  const { relay, mini } = await linked(page);
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'the link while main sends views' }).toBe('ok');
  relay.toMini = false; await relay.sent;
  await mini.evaluate(() => window.__bridge.fire('main-bye', {}));
  await expect.poll(async () => (await mini.evaluate(shown)).link, { message: 'the link after main-bye' }).toBe('away');
  const t0 = (await mini.evaluate(shown)).time;
  await page.waitForTimeout(2200);
  expect.soft((await mini.evaluate(shown)).time, 'the time after main-bye').toBe(t0);
  relay.toMini = true;
  await page.evaluate(() => App.player.setShuffle());   // a change: main sends a view
  await expect.poll(async () => (await mini.evaluate(shown)).link, { message: 'the link once a view comes' }).toBe('ok');
});

// a mini player and a main of different versions: the mini player refuses the view, says which side needs updating and
// stops its clock, instead of drawing a shape it doesn't know. A snapshot with no version is from a main before ADR-0041
test('a view of another version, or none, is refused and says which side to update', async ({ page }) => {
  const { relay, mini } = await linked(page);
  relay.toMini = false; await relay.sent;
  const last = (await got(mini)).at(-1).payload;
  const own = await mini.evaluate(shown).then((s) => s.version);
  expect(own, 'the mini player has PlayerView.VERSION').not.toBeNull();
  const send = (view) => mini.evaluate((x) => { x.clock = { ...x.clock, at: Date.now() }; window.__bridge.fire('player', x); }, view);
  const stopped = async (why) => {
    const t0 = (await mini.evaluate(shown)).time;
    await page.waitForTimeout(2200);
    expect.soft((await mini.evaluate(shown)).time, `the time stands after ${why}`).toBe(t0);
  };
  await send({ ...last, v: own + 1 });
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'a newer main' }).toBe('update-app');
  await stopped('a newer main\'s view');
  await send(last);
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'its own version again' }).toBe('ok');
  await send({ ...last, v: own - 1 });
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'an older main' }).toBe('update-hub');
  await stopped('an older main\'s view');
  // the snapshot main sent before ADR-0041
  await mini.evaluate(() => window.__bridge.fire('player', { track: { title: 'Song', artist: 'Artist', format: 'FLAC', lossless: true },
    playing: true, liked: false, pos: 20, duration: 200, durationKnown: true, buffered: [[0, 1]], received: 1, shuffle: false,
    repeat: 'off', volume: 0.5, status: 'library', statusText: '' }));
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'a snapshot with no version' }).toBe('update-hub');
  await stopped('a snapshot with no version');
  // a view drawn to the ADR's shape is still the mini player's own
  await mini.evaluate(fireView, {});
  await expect.soft.poll(async () => (await mini.evaluate(shown)).link, { message: 'a view of its own version' }).toBe('ok');
});
