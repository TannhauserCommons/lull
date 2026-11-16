// What the one painter (web/player-view.js, PlayerView.paint) and the mini player windows draw of the player view
// (ADR-0041), beyond the view reaching them (player-surfaces): the words that say what the player is doing take the room
// of what they replace and are never read out at every step; a theme's own keys, icons and seek parts stay its own; and
// the mini player's link to main and its live values say what the view says. A mini player here is a window on its own:
// what it draws is only the views a gate fires (player-windows.mjs fireView). docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import * as L from '../fixture-hub/library.mjs';
import { designs, openScreen } from './lib.mjs';
import { fireView, openMain, openMini } from './player-windows.mjs';

const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;
/* the default markup: Glasshouse has no miniPlayer slot of its own, and the strip is the default in every design */
const DEFAULT = 'glasshouse';
/* an unsaved song played as it is (never trickled), so any test may share it: the last of player-surfaces' pool, which
   that gate leaves to others */
const unsaved = (() => {
  const pool = [];
  for (const album of L.albumIds.slice(141, 147)) {
    for (const id of L.albums[album].trackIds) {
      const t = L.tracks[id];
      if (!pool.some((k) => k.title.includes(t.title) || t.title.includes(k.title))) pool.push(t);
    }
  }
  return pool.at(-1);
})();

/* a mini player window in `design` and `style` (mini: the device, or strip), with no main beside it */
const alone = (page, design = DEFAULT, style = 'mini') =>
  openMini({ page, main: [], mini: [], toMini: false, toMain: false, sent: Promise.resolve(), back: Promise.resolve() }, { design, style });

/* the player's states as main's view says them (app.js P.statusText for the words) */
const peerCopy = { where: 'soulseek', format: 'FLAC', lossless: true, peer: 'mira', received: 0.4 };
const halfway = { received: 0.4, front: 0.4, parts: [[0, 0.4]], buffered: 0.3, transferring: true };
const STATE = {
  playing: {},
  paused: { activity: 'paused', clock: { rate: 0 } },
  loading: { activity: 'loading', status: 'buffering', words: 'Buffering from mira', copy: peerCopy, transfer: halfway, clock: { rate: 0 } },
  failed: { activity: 'error', status: 'failed', words: 'No Soulseek peer shares this song right now', clock: { rate: 0 } },
};
/* the mini player in `state`: away is a playing song whose main said bye */
async function inState(mini, state, over = {}) {
  if (state === 'away') {
    await mini.evaluate(fireView, over);
    await mini.evaluate(() => window.__bridge.fire('main-bye', {}));
  } else await mini.evaluate(fireView, { ...STATE[state], ...over });
}

/* in a window: each selector's first element's box, null where there is none */
function boxesOf(sels) {
  return sels.map((s) => { const el = document.querySelector(s); if (!el) return null; const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; });
}
/* in a window: whether the selector's first element shows something: rendered, with a size, and words in it */
function showsWords(sel) {
  const el = document.querySelector(sel);
  if (!el || !el.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return false;
  const r = el.getBoundingClientRect();
  return r.width >= 1 && r.height >= 1 && el.textContent.trim() !== '';
}
const near = (a, b) => a && b && ['x', 'y', 'w', 'h'].every((k) => Math.abs(a[k] - b[k]) <= 0.5);
const sameBoxes = (a, b) => a.length === b.length && a.every((box, i) => (box === null ? b[i] === null : near(box, b[i])));
/* the boxes of `sels` with the status line taken out of the layout, then put back */
async function withoutStatus(mini, sels) {
  await mini.addStyleTag({ content: '[data-dk-status] { display: none !important; }' }).then((h) => h.evaluate((s) => s.setAttribute('data-gate', 'no-status')));
  const out = await mini.evaluate(boxesOf, sels);
  await mini.evaluate(() => document.querySelector('style[data-gate="no-status"]').remove());
  return out;
}

const PARTS = {
  mini: { player: '[data-dk-player]', title: '.device__title', artist: '.device__artist', facts: '.device__facts', wave: '[data-dk-player] .wave', times: '[data-dk-player] .wave-times' },
  strip: { player: '[data-dk-player]', title: '.dk-strip__title', artist: '.dk-strip__artist' },
};

// The words say what the player is doing when that isn't simply the song playing: they stand in for the song's facts
// (the device) or its artist (the strip) while the song loads, failed or main is away, in the same room, and take no
// room while it plays or is paused. A long artist never squeezes them out.
for (const style of ['mini', 'strip']) {
  test(`the status words take the place of the song's facts, only when there is something to say / ${style}`, async ({ page }) => {
    const mini = await alone(page, DEFAULT, style);
    const parts = PARTS[style], sels = Object.values(parts), hides = style === 'mini' ? parts.facts : parts.artist;
    const playing = {};
    for (const state of ['playing', 'paused']) {
      await inState(mini, state);
      const shown = await mini.evaluate(boxesOf, sels);
      if (state === 'playing') playing.boxes = shown;
      expect.soft(await mini.evaluate(showsWords, '[data-dk-status]'), `${state}: no status line shows`).toBe(false);
      expect.soft(sameBoxes(shown, await withoutStatus(mini, sels)), `${state}: the player, the title, the artist and the facts sit as they would with no status line`).toBe(true);
    }
    for (const state of ['loading', 'failed', 'away']) {
      await inState(mini, 'playing');
      await inState(mini, state);
      expect.soft(await mini.evaluate(showsWords, '[data-dk-status]'), `${state}: the status words show`).toBe(true);
      expect.soft(await mini.evaluate(showsWords, hides), `${state}: ${hides} gives them its place`).toBe(false);
      expect.soft(near((await mini.evaluate(boxesOf, [parts.player]))[0], playing.boxes[0]), `${state}: the player keeps its size and place`).toBe(true);
    }
    await inState(mini, 'failed', { track: { id: 't2', title: 'Song', artist: 'An Artist Whose Name Goes On And On Past Any Strip Or Screen '.repeat(3), album: 'Album' } });
    const room = await mini.evaluate(() => {
      const el = document.querySelector('[data-dk-status]');
      return { width: el.getBoundingClientRect().width, needs: el.scrollWidth, line: el.parentElement.getBoundingClientRect().width };
    });
    expect.soft(room.width, `a very long artist leaves the status words their room (${JSON.stringify(room)})`).toBeGreaterThanOrEqual(Math.min(room.needs, room.line / 2) - 0.5);
  });
}

// What changes at every step (the status words, a peer's share counting up) is never inside a live region: a screen
// reader would read the song out again at each change. The song itself is (it changes once a song).
for (const [design, style] of [...designs.map((d) => [d, 'mini']), [DEFAULT, 'strip']]) {
  test(`no live region reads out the status words or the share arrived / ${design} ${style}`, async ({ page }) => {
    const mini = await alone(page, design, style);
    await inState(mini, 'loading');
    const found = await mini.evaluate(() => ['[data-dk-status]', '[data-bind="received"]'].map((sel) => {
      const els = [...document.querySelectorAll(`[data-dk-player] ${sel}`)];
      return { sel, n: els.length, live: els.filter((el) => el.closest('[aria-live]')).length };
    }));
    expect(found[0].n, 'the window has its status line').toBeGreaterThan(0);
    if (style === 'mini') expect(found[1].n, 'the device shows the share arrived').toBeGreaterThan(0);
    for (const f of found) expect.soft(f.live, `${f.sel} inside an aria-live region`).toBe(0);
  });
}

// The status line is a line: in every design the wave keeps its height whether the line is there or not, and the line is
// no taller than one line of its words (their glyphs, or the line height its design gives them).
for (const design of designs) {
  test(`the status line is one line of words and leaves the wave its height / ${design}`, async ({ page }) => {
    const mini = await alone(page, design, 'mini');
    await inState(mini, 'failed');
    const [wave] = await mini.evaluate(boxesOf, ['[data-dk-player] .wave']);
    const [bare] = await withoutStatus(mini, ['[data-dk-player] .wave']);
    expect.soft(wave && bare && Math.abs(wave.h - bare.h), `the wave's height with the status line (${wave && wave.h}) and without (${bare && bare.h})`).toBeLessThanOrEqual(0.5);
    const line = await mini.evaluate(() => {
      const el = document.querySelector('[data-dk-player] [data-dk-status]'), range = document.createRange();
      range.selectNodeContents(el);
      const rects = [...range.getClientRects()];
      const lh = parseFloat(getComputedStyle(el).lineHeight);   // NaN for normal: the glyphs' height is the line's
      return { box: el.getBoundingClientRect().height, text: rects.length ? Math.max(...rects.map((r) => r.bottom)) - Math.min(...rects.map((r) => r.top)) : 0, lh: lh || 0 };
    });
    expect.soft(line.text, 'the status line has words').toBeGreaterThan(0);
    expect.soft(line.box, `the status line's height against one line of its words (glyphs ${line.text}, line height ${line.lh})`).toBeLessThanOrEqual(Math.max(line.text, line.lh) + 0.5);
  });
}

// Main gone (its page hidden or reloading) leaves no song playing in the mini player: the clock stops, and the keys and
// the wave say so, as they would for a pause.
for (const [design, style] of [...designs.map((d) => [d, 'mini']), [DEFAULT, 'strip']]) {
  test(`after main says bye, the mini player shows nothing playing / ${design} ${style}`, async ({ page }) => {
    const mini = await alone(page, design, style);
    await inState(mini, 'away');
    const seen = await mini.evaluate(() => {
      const player = document.querySelector('[data-dk-player]'), wave = player.querySelector('.wave'), key = player.querySelector('[data-dk="play-pause"]');
      return { link: player.getAttribute('data-link'), playing: player.getAttribute('data-playing'), wave: wave ? wave.classList.contains('is-playing') : false, key: key && key.getAttribute('aria-label') };
    });
    expect(seen.link, 'the link is away').toBe('away');
    expect.soft(seen.playing, '[data-dk-player] data-playing').toBe('false');
    expect.soft(seen.wave, 'the wave moves (.is-playing)').toBe(false);
    expect.soft(seen.key, 'the play key').toBe('Play');
  });
}

// The cover comes on its own ('player-art') and redraws the song's part; what the painter fills there (the share arrived)
// is there again at once, not at the next view, which a paused or loading song may not send for a while.
test('the share arrived stays filled when the cover comes', async ({ page }) => {
  const mini = await alone(page, DEFAULT, 'mini');
  await inState(mini, 'loading');
  const received = () => mini.evaluate(() => { const el = document.querySelector('[data-dk-player] [data-bind="received"]'); return el ? el.textContent.trim() : null; });
  expect(await received(), 'the share arrived, before the cover').toBe('40%');
  await mini.evaluate(() => window.__bridge.fire('player-art', { art: 'data:image/gif;base64,R0lGODlhAQABAAAAACw=' }));
  await expect.poll(received, { message: 'the share arrived, after the cover', timeout: 1000 }).toBe('40%');
});

// The live model's song values say nothing of a song when there is none: no song, nothing buffered of it.
test('the mini player\'s song.buffered is nothing when no song plays', async ({ page }) => {
  const mini = await alone(page, DEFAULT, 'mini');
  await mini.addScriptTag({ url: '/designs/live.js' });   // as a theme naming live values has it
  await mini.evaluate(() => Live.use(['song.pos', 'song.buffered', 'song.received']));
  await mini.evaluate(fireView, { activity: 'idle', status: 'idle', words: '', track: null, copy: null, clock: { rate: 0, pos: 0, duration: 0 },
    transfer: { received: 1, front: 1, parts: [], buffered: 0.5, transferring: false } });
  const v = await mini.evaluate(() => { const o = Live.values(); return { buffered: o['song.buffered'], received: o['song.received'], pos: o['song.pos'] }; });
  expect.soft(v.pos, 'song.pos').toBe(0);
  expect.soft(v.received, 'song.received').toBe(0);
  expect.soft([0, '', null, undefined], `song.buffered (${JSON.stringify(v.buffered)})`).toContain(v.buffered);
});

// A theme's own parts stay its own: a key it draws with CSS gets no icon put in it, an icon it draws in a repeat key keeps
// pointing where it pointed, and the player's seek parts are the player's own: a seek line outside the player isn't moved
// with the song, and a wave a theme builds into the mini player is.
test.describe('a theme\'s own parts', () => {
  test('a key the theme draws with CSS stays empty / page', async ({ page }) => {
    await openScreen(page, `/album/${L.tracks[L.savedIds[0]].albumId}`, { clock: false });
    await page.evaluate(() => {
      const own = document.createElement('div');
      own.setAttribute('data-gate', 'own-keys');
      own.innerHTML = '<button class="gate-css-key" data-bind="play" aria-label="Play"></button><button class="gate-css-key" data-bind="like" aria-label="Like"></button>';
      document.querySelector('#view').appendChild(own);
    });
    await page.evaluate((id) => App.player.playList([id], 0), L.savedIds[0]);
    await page.waitForFunction(() => App.player.view().activity === 'playing', null, { timeout: 20_000 });
    await page.evaluate(() => App.player.pause());
    await page.waitForFunction(() => document.querySelector('#pbar').getAttribute('data-playing') === 'false');
    const keys = await page.evaluate(() => [...document.querySelectorAll('[data-gate="own-keys"] button')].map((b) => ({ key: b.getAttribute('data-bind'), html: b.innerHTML })));
    expect(keys.length, 'the theme\'s keys, still in the page').toBe(2);
    for (const k of keys) expect.soft(k.html, `the theme's empty ${k.key} key`).toBe('');
  });

  test('a seek line outside the player is not the player\'s / page', async ({ page }) => {
    await openScreen(page, `/album/${L.tracks[L.savedIds[0]].albumId}`, { clock: false });
    await page.evaluate(() => {
      const own = document.createElement('div');
      own.innerHTML = '<div class="seek seek--line" data-gate="own-line"><div class="seek__track"><div class="seek__bufs"></div><div class="seek__played"></div></div></div>';
      document.querySelector('#view').appendChild(own);
    });
    await page.evaluate((id) => App.player.playList([id], 0), L.savedIds[0]);
    await page.waitForFunction(() => App.player.view().activity === 'playing' && App.player.state.pos > 1, null, { timeout: 20_000 });
    const line = await page.evaluate(() => { const el = document.querySelector('[data-gate="own-line"]'); return { pos: el.style.getPropertyValue('--pos'), bufs: el.querySelector('.seek__bufs').innerHTML, now: el.getAttribute('aria-valuenow') }; });
    expect.soft(line, 'the theme\'s seek line: no --pos, no buffered parts, no value').toEqual({ pos: '', bufs: '', now: null });
  });

  test('a repeat key\'s own icon keeps pointing where it did / mini', async ({ page }) => {
    const mini = await alone(page, DEFAULT, 'mini');
    await mini.evaluate(() => {
      const key = document.createElement('button');
      key.setAttribute('data-dk', 'repeat'); key.setAttribute('data-gate', 'own-repeat'); key.setAttribute('aria-label', 'Loop');
      key.innerHTML = '<svg aria-hidden="true"><use href="#gate-loop"></use></svg>';
      document.querySelector('[data-dk-player]').appendChild(key);
    });
    for (const repeat of ['all', 'one', 'off']) {
      await mini.evaluate(fireView, { repeat });
      const seen = await mini.evaluate(() => { const k = document.querySelector('[data-gate="own-repeat"]'); return { href: k.querySelector('use').getAttribute('href'), pressed: k.getAttribute('aria-pressed') }; });
      expect.soft(seen.href, `repeat ${repeat}: the theme's icon`).toBe('#gate-loop');
      expect.soft(seen.pressed, `repeat ${repeat}: aria-pressed still follows the view`).toBe(String(repeat !== 'off'));
    }
  });

  test('a wave the theme builds into the mini player moves with the song / mini', async ({ page }) => {
    const mini = await alone(page, DEFAULT, 'mini');
    await mini.evaluate(() => {
      const wave = document.createElement('div');
      wave.className = 'wave'; wave.setAttribute('data-gate', 'own-wave');
      wave.innerHTML = '<i class="wave__bar"></i>'.repeat(10);
      document.querySelector('[data-dk-player]').appendChild(wave);
    });
    await mini.evaluate(fireView, { clock: { pos: 100, duration: 200, rate: 0 }, activity: 'paused' });
    const wave = await mini.evaluate(() => { const w = document.querySelector('[data-gate="own-wave"]'); return { pos: w.style.getPropertyValue('--pos'), played: w.querySelectorAll('.wave__bar.is-played').length }; });
    expect.soft(Number(wave.pos), 'the theme\'s wave --pos, halfway').toBeCloseTo(0.5, 2);
    expect.soft(wave.played, 'its bars played, halfway').toBe(5);
  });

  test('the strip\'s artist is a line of its own in the song\'s words / strip', async ({ page }) => {
    const mini = await alone(page, DEFAULT, 'strip');
    await inState(mini, 'playing');
    const parent = await mini.evaluate(() => { const a = document.querySelector('.dk-strip__artist'); return a && a.parentElement.className; });
    expect(parent, '.dk-strip__artist\'s parent').toMatch(/(^|\s)dk-strip__text(\s|$)/);
  });
});

// The player's look follows the listener's intent: a song waiting for the listener to pick a copy, which the listener then
// paused, isn't playing on any surface (data-playing on the page, the player bar and the mini player).
test('a paused song waiting for a copy to be picked shows as not playing / page and mini', async ({ page, request }) => {
  const song = unsaved;
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  const relay = await openMain(page, { design: 'lull', route: `/album/${song.albumId}` });
  const mini = await openMini(relay, { design: 'lull' });
  await page.evaluate((id) => App.player.playList([id], 0), song.id);
  await page.waitForFunction((id) => App.player.current() && App.player.current().id === id, song.id);
  await request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() }, data: { topic: 'stream', data: {
    trackId: song.id, jobId: 'choose-' + song.id, status: 'choose', attempt: 0, received: 0, size: 0, choices: [], peersFound: 3, failedPeers: [] } } });
  await page.waitForFunction(() => App.player.state.status === 'choose', null, { timeout: 10_000 });
  const seen = async () => ({
    root: await page.evaluate(() => document.documentElement.getAttribute('data-playing')),
    bar: await page.evaluate(() => document.querySelector('#pbar').getAttribute('data-playing')),
    mini: await mini.evaluate(() => document.querySelector('[data-dk-player]').getAttribute('data-playing')),
  });
  await expect.poll(seen, { message: 'waiting for a pick, wanted: playing on every surface' }).toEqual({ root: 'true', bar: 'true', mini: 'true' });
  await page.evaluate(() => App.player.pause());
  await expect.poll(seen, { message: 'waiting for a pick, paused: not playing on any surface' }).toEqual({ root: 'false', bar: 'false', mini: 'false' });
});

// A key's accessible name is what it does, as it says it; its shortcut is aria-keyshortcuts, not words in its name. The
// repeat keys (the player bar's and Now Playing's) name the mode as it is, and Now Playing's shuffle and repeat pills
// either both carry a tip or neither does.
for (const design of designs) {
  test(`the repeat keys name the mode, and carry the shortcut apart / ${design}`, async ({ page }) => {
    await openScreen(page, '/now', { design, clock: false });
    await page.evaluate((id) => App.player.playList([id], 0), L.savedIds[0]);
    await page.waitForFunction(() => App.player.view().activity === 'playing', null, { timeout: 20_000 });
    for (let i = 0; i < 3; i++) {
      const mode = await page.evaluate(() => App.player.state.repeat);
      const name = { off: 'Repeat off', all: 'Repeat all', one: 'Repeat one' }[mode];
      const keys = page.locator('#pbar [data-bind="repeat"]:visible, #view [data-bind="repeat"]:visible');
      expect(await keys.count(), 'the repeat keys on show (the player bar\'s, Now Playing\'s)').toBeGreaterThan(0);
      for (const key of await keys.all()) {
        await expect.soft(key, `repeat ${mode}: the key's name`).toHaveAccessibleName(name);
        await expect.soft(key, `repeat ${mode}: its shortcut`).toHaveAttribute('aria-keyshortcuts', /(^|\s)R(\s|$)/i);
      }
      await page.evaluate(() => App.player.cycleRepeat());
    }
    const tips = await page.evaluate(() => ['shuffle', 'repeat'].map((k) => { const b = [...document.querySelectorAll(`#view [data-bind="${k}"]`)].find((el) => el.checkVisibility()); return b && b.hasAttribute('data-tip'); }));
    expect.soft(tips[0], 'Now Playing\'s shuffle and repeat keys: both with a tip, or neither (shuffle, repeat)').toBe(tips[1]);
  });
}
