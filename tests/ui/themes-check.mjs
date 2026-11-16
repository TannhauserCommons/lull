// A quick look at screens in every theme, for a UI change or while building a theme: `mise run themes:check -- <route>...`.
// Each route is drawn with a song in the player, on desktop (1440×900) and phone (390×844), day and night, in the five
// built-in designs and in every theme package in the folder LULL_THEMES_DIR names (a folder of `<id>/theme.json`
// folders; none: the built-ins only). The route `mini` draws the desktop app's mini player windows instead (the device
// and the strip, at the size the app opens them, fitted to the player as the app fits them), through the stand-in
// bridge the gates use (gates/desktop-bridge.js), a song playing and the theme handed over as one package, as the app
// gets it. Each picture runs the checks the gates run (checks.mjs: a control nothing can click, text covered or cut
// off, a feature nothing reaches, and what the menus and sheets one click opens hold; anything sticking out sideways; a
// clamp that doesn't clip; a squeezed title; console errors), which judge whether the theme works, and a contrast
// reading against what is drawn behind each line of text (checks.mjs, contrast), which is information for the author
// and never judges their look: shown in amber with the line and its ratio, for every theme and built-in design alike.
// Scrollbars show as the desktop app shows them (a phone's only while scrolling, so not there), and each scrollbar on
// the screen that lull drew because the look never did is named, in amber too (checks.mjs, plainScrollbars).
// One contact sheet shows them all: tests/ui/themes-check/index.html (or in --out's folder). It exits 1 when a check of
// function finds something, never for contrast. Flags (themes-check-args.mjs):
//   --only <id>[,<id>...]  those looks only (built in or in LULL_THEMES_DIR); --mine: LULL_THEMES_DIR's and no built-in
//   --sizes WxH[,WxH...]   the desktop at each of these sizes instead (--size: one); a laptop at 125% is 1536x864
//   --states <state>[,...] the mini player playing, paused, loading or failed (--state: one), a table each; default playing
//   --mockup <file-or-url> the mockup beside each picture, at its size; {mode}, {route} and {size} in it are filled in
//   --out <dir>            the folder the sheet and pictures go in, from the current folder; what an earlier run wrote
//                          there is replaced, and a folder holding anything else is refused
// On the fixture hub, with a copy of the theme folder, so nothing is written to it. A few minutes at most; the gates
// are the full matrix.

import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { chrome, contrast, holders, overflow, plainScrollbars, squeezed, unclippedClamps, unknownFeatures } from './checks.mjs';
import { designs as BUILT_IN, ids, openScreen, ownHub, screens } from './gates/lib.mjs';
import { fireView } from './gates/player-windows.mjs';
import { mockupUrl, outDir, parseArgs, USAGE, UsageError } from './themes-check-args.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const BRIDGE = join(here, 'gates', 'desktop-bridge.js');
const PARALLEL = 4;
const PHONE = { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true };
// the mini player's windows as the app opens them (app/src/desktop/mini.rs); each then fits its height to the player
const MINI = { device: { viewport: { width: 340, height: 560 } }, strip: { viewport: { width: 540, height: 90 } } };
// Contrast is information for the author, never a finding (ADR-0101, as the owner decided on 2026-11-09): lull's
// checks judge whether a theme works, never how it looks, so a theme that matches its design is never bent away from
// it to pass a ratio. Reported in amber with the line and its ratio, for a built-in design and a theme alike.

const fail = (message) => { console.error(`${message}\n${USAGE}`); process.exit(2); };
let opts;
try { opts = parseArgs(process.argv.slice(2)); } catch (e) { if (e instanceof UsageError) fail(e.message); throw e; }

// where the sheet goes: the run empties it first, so a folder named with --out must be empty or hold an earlier sheet
const OUT = outDir(opts.out, join(here, 'themes-check'));
if (opts.out && existsSync(OUT)) {
  const sheet = join(OUT, 'index.html');
  if (!statSync(OUT).isDirectory()) fail(`--out ${OUT} is a file, not a folder`);
  if (readdirSync(OUT).length && !(existsSync(sheet) && readFileSync(sheet, 'utf8').includes('<title>Themes check</title>'))) {
    fail(`--out ${OUT} holds other files: name an empty or new folder, or one an earlier run wrote its sheet in`);
  }
}

const desktops = opts.sizes || [{ width: 1440, height: 900 }];
const PAGE_SIZES = Object.fromEntries([
  ...desktops.map((v) => [desktops.length > 1 ? `desktop-${v.width}x${v.height}` : 'desktop', { viewport: v }]),
  ['phone', PHONE],
]);
const slugOf = (path) => path.replace(/[^\w]+/g, '-').replace(/^-|-$/g, '') || 'home';
// name: a key screen's, `mini`, or the path's slug (the mockup's {route}); path: what the web UI opens
const routes = opts.routes.map((r) => {
  if (r === 'mini') return (opts.states || ['playing']).map((state) => ({ name: 'mini', mini: true, state, sizes: MINI }));
  const path = screens[r] || (r.startsWith('/') ? r : '/' + r);
  return { name: screens[r] ? r : slugOf(path), path, sizes: PAGE_SIZES };
}).flat();
// the mini player's route as said: with its state when --states asked for them
const miniSaid = (route) => (opts.states ? `mini, ${route.state}` : 'mini');

/* the listener's themes: a copy of each package in LULL_THEMES_DIR, with its modes */
const copy = mkdtempSync(join(tmpdir(), 'lull-themes-check-'));
const mine = [];
const from = process.env.LULL_THEMES_DIR;
if (from) {
  for (const entry of readdirSync(from, { withFileTypes: true })) {
    const manifest = join(from, entry.name, 'theme.json');
    if (!entry.isDirectory() || !existsSync(manifest)) continue;
    try {
      const m = JSON.parse(readFileSync(manifest, 'utf8'));
      if (m.id !== entry.name) { console.warn(`skipped ${entry.name}: its theme.json says id ${JSON.stringify(m.id)}`); continue; }
      cpSync(join(from, entry.name), join(copy, entry.name), { recursive: true });
      mine.push({ id: m.id, modes: Array.isArray(m.modes) && m.modes.length ? m.modes : ['day', 'night'], builtIn: false });
    } catch (e) { console.warn(`skipped ${entry.name}: ${e.message}`); }
  }
} else if (!opts.only) console.warn(`LULL_THEMES_DIR is not set: ${opts.mine ? 'no themes of yours' : 'the five built-in designs only'}`);

/* the looks to draw: all, yours (--mine) or the ones named (--only), in the order named */
const builtIns = BUILT_IN.map((id) => ({ id, modes: ['day', 'night'], builtIn: true }));
let looks = [...builtIns, ...mine];
if (opts.mine) looks = mine;
if (opts.only) {
  const unknown = opts.only.filter((id) => !looks.some((d) => d.id === id));
  if (unknown.length) { rmSync(copy, { recursive: true, force: true }); fail(`no such look: ${unknown.join(', ')} (there are ${looks.map((d) => d.id).join(', ')})`); }
  looks = [...new Set(opts.only)].map((id) => looks.find((d) => d.id === id));
}
if (!looks.length) { rmSync(copy, { recursive: true, force: true }); fail('--mine: no theme packages in LULL_THEMES_DIR'); }

const jobs = routes.flatMap((route) => looks.flatMap((d) => Object.keys(route.sizes).flatMap((size) =>
  d.modes.map((mode) => ({ route, design: d.id, builtIn: d.builtIn, size, mode })))));
const slug = (j) => `${j.route.mini ? slugOf(miniSaid(j.route)) : slugOf(j.route.path)}--${j.design}--${j.size}--${j.mode}`;

if (existsSync(OUT)) for (const entry of readdirSync(OUT)) rmSync(join(OUT, entry), { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const started = Date.now();
const hub = await ownHub({ FIXTURE_THEMES: copy });
// scrollbars drawn as the app draws them: Playwright starts Chromium with --hide-scrollbars, and no picture had one, while
// the desktop app (WebView2, and the mini player's windows) always shows them, so a theme went out with bars nobody saw.
// A phone's are overlay bars that show only while scrolling: hidden there (scrollbarsAsDrawn), as on the mockups.
const browser = await chromium.launch({ ignoreDefaultArgs: ['--hide-scrollbars'] });
async function hideScrollbars(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setScrollbarsHidden', { hidden: true });
}
const scrollbarsAsDrawn = (page, size) => (size === 'phone' ? hideScrollbars(page) : null);

/* the web UI in a look and mode, with a song in the player: the fixture album's first */
async function playFirst(page, look, errors) {
  await openScreen(page, `/album/${ids.album}`, look);
  await page.evaluate(() => { const row = document.querySelector('#view .trow[data-track-id]'); return row && App.player.playTrack(row.getAttribute('data-track-id'), [row.getAttribute('data-track-id')]); });
  await page.waitForFunction(() => App.player.current(), null, { timeout: 5000 }).catch(() => errors.push('no song started'));
}
/* the player says the song plays (ADR-0041's view), on the route asked, and what the route brought in has finished
   coming in (a sheet sliding up, a hero fading in: every animation that ends); a web UI from before the view had no
   view, and that one is given the time it always was first */
const hasView = (page) => page.evaluate(() => typeof App.player.view === 'function');
async function settled(page, path, errors) {
  if (!(await hasView(page))) await page.waitForTimeout(400);
  else await page.waitForFunction((p) => App.current && App.current.path === p && App.player.view().activity === 'playing', path, { timeout: 10000 })
    .catch(() => errors.push('the song never played on ' + path));
  await page.evaluate(() => Promise.race([
    Promise.all(document.getAnimations().filter((a) => a.effect && a.effect.getComputedTiming().endTime !== Infinity).map((a) => a.finished.catch(() => {}))),
    new Promise((ok) => setTimeout(ok, 5000)),
  ]));
}

/* what the checks find in a picture as it stands, the picture, and the contrast against what is drawn */
async function judge(page, job, errors, { seeThrough = false } = {}) {
  await page.evaluate(() => document.fonts.ready);
  if (job.route.mini || !(await hasView(page))) await page.waitForTimeout(300);
  // a built-in design's features findings the owner has yet to decide on (gates/features-known.json) aren't counted
  const known = (found) => (job.builtIn ? unknownFeatures(found, { design: job.design, width: page.viewportSize().width, screen: job.route.name }) : found);
  const found = [...known(await chrome(page)), await overflow(page), await unclippedClamps(page), await squeezed(page)].filter(Boolean).map(String);
  const file = slug(job) + (seeThrough ? '.png' : '.jpg');
  // a see-through window keeps its transparency, so the sheet shows what the desktop would show through
  await page.screenshot(seeThrough ? { path: join(OUT, file), omitBackground: true } : { path: join(OUT, file), type: 'jpeg', quality: 72 });
  const problems = [...found, ...errors], notes = [];
  const low = await contrast(page, { seeThrough });
  if (low) notes.push('contrast ' + low);
  const plain = await plainScrollbars(page);
  if (plain) notes.push(plain);
  // last, as it opens and closes them: the menus and sheets one click opens hold what web/designs/features.json says
  problems.push(...known(await holders(page)));
  return { ...job, file, viewport: page.viewportSize(), problems, notes };
}

/* the output's decks (new Audio()), kept where the page can reach them */
function keepDecks() {
  const Made = window.Audio;
  window.__decks = [];
  window.Audio = function (src) { const el = src === undefined ? new Made() : new Made(src); window.__decks.push(el); return el; };
  window.Audio.prototype = Made.prototype;
}
/* the song held a third of the way in while it plays, so every run draws the same moment: once what the route reads
   has come in (no skeleton left), its decks stand still (playbackRate 0, not a pause: the player still says it plays)
   and the player seeks there in two steps, a second short and then there, each followed by the screens' tick, so what
   follows the song on them (synced lyrics take two: one to start, one to follow) has caught up with it */
async function holdSong(page, errors) {
  await page.waitForFunction(() => !document.querySelector('#view .skel, #view [aria-busy="true"]'), null, { timeout: 5000 }).catch(() => {});
  await page.evaluate(() => { for (const el of window.__decks) el.playbackRate = 0; window.__ticks = 0; App.on('tick', () => window.__ticks++); });
  for (const back of [1, 0]) {
    const ticks = await page.evaluate(() => window.__ticks);
    await page.evaluate((b) => App.player.seek(Math.floor(App.player.duration() / 3) - b), back);
    await page.waitForFunction(([b, n]) => Math.abs(App.player.state.pos - (Math.floor(App.player.duration() / 3) - b)) < 0.05 && App.player.state.playing && window.__ticks > n,
      [back, ticks], { timeout: 10000 }).catch(() => errors.push('the song never held a third of the way in'));
  }
}

/* one picture of a web UI route: the route in that design, size and mode, with a song in the player, held a third of
   the way in */
async function shootPage(job) {
  const context = await browser.newContext({ baseURL: hub.url, reducedMotion: 'reduce', ...job.route.sizes[job.size] });
  const page = await context.newPage();
  const errors = listen(page);
  try {
    await scrollbarsAsDrawn(page, job.size);
    await page.addInitScript(keepDecks);
    await playFirst(page, { design: job.design, theme: job.mode }, errors);
    await page.evaluate((r) => { location.hash = '#' + r; }, job.route.path);
    await settled(page, job.route.path, errors);
    await holdSong(page, errors);
    await settled(page, job.route.path, errors);
    return await judge(page, job, errors);
  } catch (e) {
    return { ...job, problems: [...errors, 'could not draw it: ' + e.message.split('\n')[0]], notes: [] };
  } finally { await context.close(); }
}

/* main, as the app runs the web UI, for each look once: the song it plays, as the view and cover it sends the mini
   player when one opens, and the look as it hands it to the app (a package for a listener's theme, and the design it
   extends). The design's own: the song and its cover are the same in every look */
const mains = new Map();
const mainFor = (design) => { if (!mains.has(design)) mains.set(design, readMain(design)); return mains.get(design); };
async function readMain(design) {
  const context = await browser.newContext({ baseURL: hub.url, reducedMotion: 'reduce', viewport: { width: 1280, height: 800 } });
  const page = await context.newPage();
  try {
    await page.addInitScript({ path: BRIDGE });
    const errors = [];
    await playFirst(page, { design, theme: 'day' }, errors);
    if (errors.length) throw new Error(errors.join('; '));
    await page.evaluate(() => window.__bridge.fire('mini', { action: 'hello' }));
    await page.waitForFunction(() => window.__bridge.last.player && window.__bridge.last.player.track, null, { timeout: 5000 });
    await page.waitForFunction(() => (window.__bridge.last['player-art'] || {}).art, null, { timeout: 5000 }).catch(() => { /* the fallback cover */ });
    if (!BUILT_IN.includes(design)) {
      await page.waitForFunction((d) => { const s = window.__bridge.last.set_theme_package; return s && s.package && s.package.manifest.id === d; }, design, { timeout: 10000 });
    }
    return await page.evaluate(() => {
      const l = window.__bridge.last;
      const t = App.player.current();
      return { snap: l.player, song: { id: t.id, title: t.title, artist: t.artist, album: t.album || '', duration: App.player.duration() },
        art: (l['player-art'] || {}).art || null, pkg: (l.set_theme_package || {}).package || null, base: (l.appearance || {}).base || null };
    });
  } finally { await context.close(); }
}

/* one picture of a mini player window: the app's page (desktop/wrapper.html) with no hub beside it, the look main handed
   over, main's song a third of the way in, in the state asked (--states) */
async function shootMini(job) {
  const context = await browser.newContext({ baseURL: hub.url, reducedMotion: 'reduce', ...MINI[job.size] });
  const page = await context.newPage();
  const errors = listen(page);
  try {
    const main = await mainFor(job.design);
    await page.addInitScript({ path: BRIDGE });
    await page.addInitScript((p) => { window.__bridge.answers.theme_package = (args) => (p && args.id === p.manifest.id ? p : null); }, main.pkg);
    await page.route('**/api/**', (r) => r.fulfill({ status: 404, body: 'not here' }));
    const hash = new URLSearchParams({ view: job.size === 'strip' ? 'strip' : 'mini', design: job.design, theme: job.mode });
    if (main.base) hash.set('base', main.base);
    await page.goto(`/desktop/wrapper.html#${hash}`);
    await page.waitForFunction((d) => !document.documentElement.hasAttribute('data-booting') && window.Designs && Designs.current === d &&
      document.querySelector('[data-dk-player]'), job.design, { timeout: 10000 });
    const player = playerIn(main, job.route.state);
    const live = liveIn(main, player);
    if (main.snap.clock) await page.evaluate(fireView, asView(player, live));
    else await page.evaluate((s) => window.__bridge.fire('player', s), asSnapshot(player, live));   // before the view
    await page.evaluate((art) => window.__bridge.fire('player-art', { art }), main.art);
    await page.evaluate(() => document.fonts.ready);
    // the window fits its height to the player (desktop.js, fitWindow), as the app's does
    const fit = await page.evaluate(() => window.__bridge.last.setSize);
    if (fit && fit.height) await page.setViewportSize({ width: MINI[job.size].viewport.width, height: Math.ceil(fit.height) });
    return await judge(page, job, errors, { seeThrough: true });
  } catch (e) {
    return { ...job, problems: [...errors, 'could not draw it: ' + e.message.split('\n')[0]], notes: [] };
  } finally { await context.close(); }
}

/* What every mini player picture shows, whichever web UI draws it: one record per state, taken from nothing a run may
   vary. The song and its length are main's (the fixture album's first song; the theme's live values are main's too),
   and everything else is fixed here: a third of the way in, the copy, the volume, the keys' states and the words. A web
   UI with the view gets it as a view (fireView), one from before the view as main's snapshot, so the two windows are
   handed the same player, and a picture that differs between two runs differs because the product changed */
const STATES = {
  playing: { activity: 'playing', intent: true, status: 'library', words: 'Playing from your library', received: 1 },
  paused: { activity: 'paused', intent: false, status: 'library', words: 'Playing from your library', received: 1 },
  loading: { activity: 'loading', intent: true, status: 'buffering', words: 'Buffering from a peer', received: 0.4 },
  failed: { activity: 'error', intent: false, status: 'failed', words: 'No Soulseek peer shares this song right now.', received: 1 },
};
function playerIn(main, state = 'playing') {
  const { duration, ...track } = main.song;
  return { track, format: 'FLAC', lossless: true, duration, pos: Math.floor(duration / 3), ...STATES[state],
    liked: false, saved: true, shuffle: false, repeat: 'off', volume: 0.8 };
}
/* the live values the theme names, as main read them, with what the record above fixes said as the record says it. The
   song's place is no part of them: the mini player works it out (ADR-0041; before the view, it moved them on itself) */
const PLACE = ['song.pos', 'song.elapsed', 'song.buffered', 'song.received'];
function liveIn(main, p) {
  if (!main.snap.live) return undefined;
  const live = { ...main.snap.live };
  for (const k of PLACE) delete live[k];
  const fixed = { 'player.playing': p.intent, 'player.status': p.status, 'player.status-text': p.words, 'player.volume': p.volume,
    'player.repeat': p.repeat, 'player.shuffle': p.shuffle, 'player.muted': false, 'song.saved': p.saved, 'song.liked': p.liked,
    'copy.peer': p.received < 1 ? 'a peer' : '' };
  for (const [k, v] of Object.entries(fixed)) if (k in live) live[k] = v;
  return live;
}
function asView(p, live) {
  const arriving = p.received < 1;
  return {
    activity: p.activity, status: p.status, words: p.words, since: null, intent: p.intent, track: p.track,
    copy: { where: arriving ? 'soulseek' : 'library', format: p.format, lossless: p.lossless, peer: arriving ? 'a peer' : null, received: p.received },
    clock: { pos: p.pos, at: Date.now(), rate: 0, duration: p.duration, known: true },
    transfer: { received: p.received, front: p.received, parts: [[0, p.received]], buffered: arriving ? 0.3 : 1, transferring: arriving },
    seekWait: null, liked: p.liked, saved: p.saved, shuffle: p.shuffle, repeat: p.repeat, volume: p.volume, muted: false, crossfading: false,
    ...(live ? { live } : {}),
  };
}
function asSnapshot(p, live) {
  return {
    track: { title: p.track.title, artist: p.track.artist, format: p.format, lossless: p.lossless }, playing: p.intent,
    liked: p.liked, saved: p.saved, pos: p.pos, duration: p.duration, durationKnown: true,
    buffered: [[0, p.received]], front: p.received, received: p.received, shuffle: p.shuffle, repeat: p.repeat, volume: p.volume,
    status: p.status, statusText: p.words, ...(live ? { live } : {}),
  };
}

function listen(page) {
  const errors = [];
  page.on('pageerror', (e) => errors.push('page error: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error' && !m.text().startsWith('Failed to load resource')) errors.push('console: ' + m.text()); });
  return errors;
}

/* the mockup beside each picture, at its size: one shot per address and size */
async function shootMockup(shot) {
  const context = await browser.newContext({ ...shot.options, viewport: shot.viewport });
  const page = await context.newPage();
  try {
    await hideScrollbars(page);   // a mockup is the picture intended: its page's own scrollbars aren't in it
    await page.goto(shot.url, { waitUntil: 'load', timeout: 15000 });
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(300);
    await page.screenshot({ path: join(OUT, shot.file), type: 'jpeg', quality: 72 });
  } catch (e) {
    shot.error = e.message.split('\n')[0];
  } finally { await context.close(); }
}

async function inParallel(list, fn, tick) {
  let next = 0;
  await Promise.all(Array.from({ length: PARALLEL }, async () => {
    while (next < list.length) { const item = list[next++]; tick(await fn(item)); }
  }));
}

const results = [];
const shots = new Map();
try {
  await inParallel(jobs, (job) => (job.route.mini ? shootMini(job) : shootPage(job)), (r) => {
    results.push(r);
    process.stdout.write(r.problems.length ? 'x' : '.');
  });
  if (opts.mockup) {
    const sizeWord = (s) => (s.startsWith('desktop') ? 'desktop' : s);
    for (const r of results) {
      if (!r.file) continue;
      const url = mockupUrl(opts.mockup, { mode: r.mode, route: r.route.name, size: sizeWord(r.size) });
      const key = `${url} ${r.viewport.width}x${r.viewport.height}`;
      if (!shots.has(key)) shots.set(key, { url, viewport: r.viewport, options: r.size === 'phone' ? PHONE : {}, file: `mockup-${shots.size + 1}.jpg` });
      r.mockup = shots.get(key);
    }
    await inParallel([...shots.values()], shootMockup, () => process.stdout.write('m'));
  }
} finally { await browser.close(); hub.close(); rmSync(copy, { recursive: true, force: true }); }
process.stdout.write('\n');

/* the contact sheet: a row per look, a column per size and mode, under each route; what a check found under its
   picture, and the mockup beside it */
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const failed = results.filter((r) => r.problems.length);
const noted = results.filter((r) => r.notes.length);
const sizeLabel = (route, size) => { const v = route.sizes[size].viewport; return `${size.replace(/-\d+x\d+$/, '')} ${v.width}×${v.height}`; };
const picture = (file, alt, see) => `<a href="${file}"><img src="${file}" alt="${esc(alt)}" loading="lazy"${see ? ' class="see"' : ''}></a>`;
const cell = (r) => {
  if (!r) return '<td class="none">no such mode</td>';
  const alt = `${r.design}, ${r.size}, ${r.mode}${r.route.mini && opts.states ? ', ' + r.route.state : ''}`, see = r.route.mini;
  const theme = r.file ? picture(r.file, alt, see) : '';
  const shown = !r.mockup ? theme : `<div class="pair"><figure>${theme}<figcaption>theme</figcaption></figure><figure>${r.mockup.error
    ? `<p class="bad">could not open the mockup: ${esc(r.mockup.error)}</p>` : picture(r.mockup.file, 'mockup, ' + alt)}<figcaption>mockup</figcaption></figure></div>`;
  const list = (cls, items) => (items.length ? `<ul class="${cls}">${items.map((p) => `<li>${esc(p)}</li>`).join('')}</ul>` : '');
  return `<td class="${r.problems.length ? 'bad' : ''}">${shown}${list('found', r.problems)}${list('noted', r.notes)}</td>`;
};
const table = (route) => {
  const cols = Object.keys(route.sizes).flatMap((size) => ['day', 'night'].map((mode) => ({ size, mode })));
  const title = route.mini ? `${miniSaid(route)}: the desktop app's mini player, device and strip` : route.path;
  return `<h2>${esc(title)}</h2><table><thead><tr><th></th>${cols.map((c) => `<th>${esc(sizeLabel(route, c.size))}, ${c.mode}</th>`).join('')}</tr></thead><tbody>
${looks.map((d) => `<tr><th scope="row">${esc(d.id)}${d.builtIn ? '<br><small>built in</small>' : ''}</th>${cols.map((c) =>
    cell(results.find((r) => r.route === route && r.design === d.id && r.size === c.size && r.mode === c.mode))).join('')}</tr>`).join('\n')}
</tbody></table>`;
};
const sizesSaid = Object.keys(PAGE_SIZES).map((s) => sizeLabel({ sizes: PAGE_SIZES }, s)).join(', ');
writeFileSync(join(OUT, 'index.html'), `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Themes check</title>
<style>
  :root { color-scheme: light dark; --bg: #fff; --fg: #1d1d1f; --line: #d0d0d6; --bad: #c62828; --note: #8a5a00; }
  @media (prefers-color-scheme: dark) { :root { --bg: #17171a; --fg: #ececf0; --line: #3a3a42; --bad: #ff6b6b; --note: #f0b44c; } }
  body { margin: 16px; background: var(--bg); color: var(--fg); font: 14px/1.4 system-ui, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 4px; } h2 { font-size: 16px; margin: 24px 0 8px; }
  table { border-collapse: collapse; width: 100%; table-layout: fixed; }
  th, td { border: 1px solid var(--line); padding: 6px; vertical-align: top; text-align: left; }
  th[scope=row] { width: 110px; } img { display: block; width: 100%; height: auto; }
  img.see { width: auto; max-width: 100%; background: repeating-conic-gradient(#c8c8cc 0 25%, #f4f4f6 0 50%) 0 0 / 16px 16px; }
  .pair { display: grid; grid-template-columns: 1fr 1fr; gap: 6px; } figure { margin: 0; min-width: 0; }
  figcaption { font-size: 11px; color: GrayText; margin-top: 2px; }
  td.bad { outline: 2px solid var(--bad); outline-offset: -2px; } ul { margin: 6px 0 0; padding-left: 18px; font-size: 12px; }
  ul.found, p.bad { color: var(--bad); } ul.noted { color: var(--note); }
  td.none { color: GrayText; font-size: 12px; }
</style>
<h1>Themes check</h1>
<p>${esc(sizesSaid)}${routes.some((r) => r.mini) ? '; the mini player at the width the app opens it, its height fitted to the player, over a checkerboard where the desktop shows through' : ''}. ${results.length} pictures, ${failed.length ? `<strong style="color:var(--bad)">${failed.length} with something to look at</strong>` : 'nothing found by the checks'}; ${Math.round((Date.now() - started) / 1000)} s. ${opts.mine ? 'The themes in LULL_THEMES_DIR' : opts.only ? 'The looks named' : 'Built-in designs, then the themes in LULL_THEMES_DIR'}, with a song in the player.${noted.length ? ` <span style="color:var(--note)">Contrast, and a scrollbar the look left to lull, are reported in this colour for the author, never counted.</span>` : ''}${opts.mockup ? ` The mockup, ${esc(opts.mockup)}, beside each picture at its size.` : ''}</p>
${routes.map(table).join('\n')}
`);

/* what each look's mini pictures were handed, beside the sheet: two runs whose inputs.json differ drew different players */
if (mains.size) {
  const inputs = {};
  for (const [design, got] of mains) {
    const main = await got.catch(() => null);
    if (!main) continue;
    inputs[design] = { states: Object.fromEntries((opts.states || ['playing']).map((st) => { const p = playerIn(main, st); return [st, { ...p, live: liveIn(main, p) || null }]; })),
      art: main.art ? createHash('sha256').update(main.art).digest('hex').slice(0, 16) : null };
  }
  writeFileSync(join(OUT, 'inputs.json'), JSON.stringify(inputs, null, 1));
}
const where = (r) => `${r.route.mini ? miniSaid(r.route) : r.route.path} ${r.design} ${r.size} ${r.mode}`;
console.log(`${results.length} pictures in ${Math.round((Date.now() - started) / 1000)} s; ${failed.length ? failed.length + ' with something to look at' : 'nothing found'}`);
for (const r of failed) console.log(`  ${where(r)}: ${r.problems.join('; ')}`);
if (noted.length) console.log(`for you to weigh, not counted (contrast and lull's own scrollbars, information for the author):\n${noted.map((r) => `  ${where(r)}: ${r.notes.join('; ')}`).join('\n')}`);
const unopened = [...shots.values()].filter((s) => s.error);
if (unopened.length) console.log(`mockup not drawn:\n${unopened.map((s) => `  ${s.url}: ${s.error}`).join('\n')}`);
console.log('contact sheet: ' + pathToFileURL(join(OUT, 'index.html')).href);
process.exit(failed.length ? 1 : 0);
