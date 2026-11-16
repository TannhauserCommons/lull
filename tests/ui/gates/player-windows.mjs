// The desktop app's two windows in one browser, for the gates on the player view (ADR-0041): `main` (lull's page) and
// a mini player window (wrapper.html#view=mini|strip), each with the stand-in Tauri bridge (desktop-bridge.js), joined
// as the app joins them. What main emits reaches the mini player as an event, and the mini player's 'mini' events reach
// main, in the order they were sent, through this process: the mini player only ever sees views main really made.
// relay.toMini / relay.toMain cut a direction (main gone, or stalled); relay.main and relay.mini log what each sent.

import { fileURLToPath } from 'node:url';
import { openScreen } from './lib.mjs';

const bridge = fileURLToPath(new URL('./desktop-bridge.js', import.meta.url));

/* main's page, before any of its scripts: the app's window, a page it can hide as the app's window is hidden
   (document.hidden and visibilityState both, and the visibilitychange event), the output's decks, and every event it
   sends across the bridge passed on */
function mainWindow() {
  window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' };
  let hidden = false;
  Object.defineProperty(document, 'hidden', { configurable: true, get: () => hidden });
  Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (hidden ? 'hidden' : 'visible') });
  window.__hide = (h) => { hidden = h; document.dispatchEvent(new Event('visibilitychange')); };
  // the <audio> decks the engine plays (never in the document): a gate pauses the playing one as the system would
  const play = HTMLMediaElement.prototype.play;
  window.__played = new Set();
  HTMLMediaElement.prototype.play = function () { window.__played.add(this); return play.call(this); };
  const ev = window.__TAURI__.event, emit = ev.emit, emitTo = ev.emitTo;
  ev.emit = (name, p) => { window.__toMini(name, p === undefined ? null : p); return emit(name, p); };
  ev.emitTo = (to, name, p) => { if (to !== 'main') window.__toMini(name, p === undefined ? null : p); return emitTo(to, name, p); };
}
/* the mini player's page: what it sends main ('mini' events: hello, its keys) passed on */
function miniWindow() {
  const ev = window.__TAURI__.event, emit = ev.emit, emitTo = ev.emitTo;
  ev.emit = (name, p) => { window.__toMain(name, p === undefined ? null : p); return emit(name, p); };
  ev.emitTo = (to, name, p) => { window.__toMain(name, p === undefined ? null : p); return emitTo(to, name, p); };
}
/* an event as Rust or the other window delivers it, kept in the receiving page's own log (__got) */
function deliver([name, payload]) {
  (window.__got = window.__got || []).push({ name, payload, at: Date.now() });
  window.__bridge.fire(name, payload);
}

/** Main's page at `route` in `design`, joined to a relay a mini player opens on later (openMini). No fixed clock: the
    view's clock is in real milliseconds, and the mini player reads it against its own Date.now(). */
export async function openMain(page, { design = 'lull', route = '/home' } = {}) {
  const relay = { toMini: true, toMain: true, main: [], mini: [], miniPage: null, page, sent: Promise.resolve(), back: Promise.resolve() };
  await page.exposeFunction('__toMini', (name, payload) => {
    relay.main.push({ name, payload, at: Date.now() });
    const mini = relay.miniPage;
    if (relay.toMini && mini) relay.sent = relay.sent.then(() => mini.evaluate(deliver, [name, payload]).catch(() => {}));
  });
  await page.addInitScript({ path: bridge });
  await page.addInitScript(mainWindow);
  await openScreen(page, route, { design, clock: false });
  return relay;
}

/** A mini player window in `style` (mini: the device, or strip), joined to main's relay. It says hello as it opens. */
export async function openMini(relay, { design = 'lull', style = 'mini' } = {}) {
  const mini = await relay.page.context().newPage();
  await mini.exposeFunction('__toMain', (name, payload) => {
    relay.mini.push({ name, payload, at: Date.now() });
    if (relay.toMain && name === 'mini') relay.back = relay.back.then(() => relay.page.evaluate(deliver, [name, payload]).catch(() => {}));
  });
  await mini.addInitScript({ path: bridge });
  await mini.addInitScript(miniWindow);
  await mini.route('**/api/**', (r) => r.fulfill({ status: 404, body: 'not here' }));   // the app serves it: no hub beside it
  relay.miniPage = mini;
  await mini.goto(`/desktop/wrapper.html#view=${style}&design=${design}&theme=day&opaque=1`);
  await mini.waitForFunction(() => document.querySelector('[data-dk-player]'));
  return mini;
}

/** The views main sent the mini player, oldest first. */
export const views = (relay) => relay.main.filter((e) => e.name === 'player').map((e) => e.payload);
/** The views the mini player got, as { payload, at }, in the mini player. */
export const got = (mini) => mini.evaluate(() => (window.__got || []).filter((e) => e.name === 'player'));
/** What the mini player shows of the player, read in its window. */
export function shown() {
  const player = document.querySelector('[data-dk-player]');
  const text = (sel) => { const el = player && player.querySelector(sel); return el ? el.textContent.trim() : null; };
  const clock = text('[data-dk-pos]');
  const [m, s] = (clock || '').split(':').map(Number);
  return {
    link: player ? player.getAttribute('data-link') : null,
    time: clock == null ? null : m * 60 + s,
    songPos: player ? parseFloat(player.style.getPropertyValue('--song-pos')) : null,
    title: text('[data-dk-title]') ?? text('[data-dk-now] .device__title'),
    status: text('[data-dk-status]'),
    chip: text('.q-chip'),
    received: text('[data-bind="received"]'),   // the share arrived, as the page's hook says it ("42%")
    version: window.PlayerView ? window.PlayerView.VERSION : null,
  };
}

/** A view as ADR-0041 shapes it, for a gate that draws one by hand in a mini player's window (page.evaluate(fireView,
    over)): a song playing from the library at 0:20 of 3:20, its clock set as it is sent. `over` replaces fields, and
    clock's own fields one by one. */
export function fireView(over) {
  const base = {
    v: window.PlayerView ? window.PlayerView.VERSION : null, activity: 'playing', status: 'library', words: 'Playing from your library',
    track: { id: 't1', title: 'Song', artist: 'Artist', album: 'Album' },
    copy: { where: 'library', format: 'FLAC', lossless: true, peer: null, received: 1 },
    clock: { pos: 20, at: Date.now(), rate: 1, duration: 200, known: true },
    transfer: { received: 1, front: 1, parts: [[0, 1]], transferring: false }, seekWait: null,
    liked: false, saved: true, shuffle: false, repeat: 'off', volume: 0.5, muted: false, crossfading: false,
  };
  window.__bridge.fire('player', { ...base, ...over, clock: { ...base.clock, ...(over && over.clock) } });
}
