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
import { designs } from './lib.mjs';
import { fireView, got, openMain, openMini, shown, views } from './player-windows.mjs';

const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;
const LOADING = ['finding', 'queued', 'buffering', 'stalled'];

/* the catalogue's unsaved albums (Soulseek songs: a transfer the fixture can trickle) one per design, so no two tests
   trickle the same song (the feed's first new release, which other gates trickle, is the last of them: not used here);
   their songs whose titles neither contain nor are in another's, so a title found on a surface is that song's */
const unsaved = L.albumIds.slice(140, 148);
function songsOf(album) {
  const kept = [];
  for (const id of L.albums[album].trackIds) {
    const t = L.tracks[id];
    if (!kept.some((k) => k.title.includes(t.title) || t.title.includes(k.title))) kept.push(t);
  }
  return kept;
}
/* what Home's lead shows of the queue, per design (its homeLead slot): lull's desk widgets show neither */
const LEAD = { lull: [], petal: ['next'], skydeck: ['title', 'next'], starlight: ['next'], glasshouse: ['title', 'next'] };

/* one page's own events on the fixture hub; an album's page, so the app knows its songs */
async function mainAt(page, design, route) {
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  return openMain(page, { design, route });
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
      const album = unsaved[i], songs = songsOf(album);
      const relay = await mainAt(page, design, `/album/${album}`);
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
      expect.soft(showsShare(m.percents, share.last), `the mini player shows the last view's share, ${receivedOf(share.last)}%: it shows ${m.percents.join(', ')}%`).toBe(true);

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
      const album = unsaved[i], songs = songsOf(album).slice(3, 6);
      const relay = await mainAt(page, design, `/album/${album}`);
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
    const album = unsaved[5], songs = songsOf(album);
    const short = songs.reduce((a, b) => (b.duration < a.duration ? b : a)), slow = songs.find((t) => t !== short);
    const relay = await mainAt(page, 'lull', `/album/${album}`);
    await trickle(page, slow.id, 200);
    const mini = await openMini(relay, { design: 'lull', style: 'strip' });
    await page.evaluate((id) => App.player.playList([id], 0), short.id);
    await playing(page);
    await hide(page);
    await page.waitForTimeout(1000);

    const v = await mainVersion(page);
    expect.soft(v, 'main has PlayerView.VERSION').not.toBeNull();
    expect.soft(views(relay).filter((x) => x.v !== v || !x.clock).length, 'views without the wire version or a clock').toBe(0);
    // a hundredth of the shortest song is under 4.2 s: 5 s moves it on at least one
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

// The system pausing the output (another app takes the audio, headphones out) is a pause: the listener's intent follows
// the output, so the bar shows Play, and the mini player, Connect and the tray hear a pause.
test('a pause the system makes shows as a pause', async ({ page }) => {
  const song = L.savedIds[0];
  await mainAt(page, 'lull', `/album/${L.tracks[song].albumId}`);
  await page.evaluate((id) => App.player.playList([id], 0), song);
  await playing(page);
  const paused = await page.evaluate(() => {
    const deck = [...window.__played].find((el) => !el.paused && /\/api\/(library\/)?stream\//.test(el.src));
    if (deck) deck.pause();
    return !!deck;
  });
  expect(paused, 'found the deck the engine plays').toBe(true);
  await expect.poll(() => page.evaluate(() => App.player.state.playing), { message: 'the player\'s intent' }).toBe(false);
  await expect.poll(() => page.evaluate(() => document.querySelector('#pbar [data-bind="play"]').getAttribute('aria-label')),
    { message: 'the bar\'s play key' }).toBe('Play');
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
