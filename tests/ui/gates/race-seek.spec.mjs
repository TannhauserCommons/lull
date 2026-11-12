// Acceptance tests for ADR-0018: the builder must not edit this file.
//
// Seeking through the better-copy race (ADR-0018, S3-S9). The owner: "make sure everything works including seeking".
// While the hub races peers for a better copy, cancels the losers and hands the player the winner, the listener's place
// holds: no jump back to 0, no restart (one load of the song), no seek the listener didn't make, and a seek past what
// arrived waits and lands as it does today (kept-seek.spec.mjs). These guard what works now; the race must keep it so.
//
// The hub's half is the fixture's: `stream` events emitted as the race would send them (no offer while lanes race; the
// winner offered ready; an offer withdrawn when its lane is cancelled), and its stand-in for POST /switch. The output is
// lib.mjs's fakeOutput (the Android app's native player), with every load recorded; the desktop app's mini player is
// checked with the page's own output, as the desktop app has no native one. docs/context/quality.md#ui-gates.

import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { fakeOutput, openScreen } from './lib.mjs';

const bridge = fileURLToPath(new URL('./desktop-bridge.js', import.meta.url));
const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;
const SIZE = 4_000_000;
const PATH = 'Music\\Artist\\Album\\01 - Song.mp3';

/* every load of a song into the output (a restart) and, when asked, a switch that waits for the test to let it commit */
function recordLoads() {
  const p = window.LullNative.player, load = p.load;
  window.__out.loads = [];
  p.load = (item, o) => { window.__out.loads.push({ url: item.url, at: (o && o.at) || 0 }); return load(item, o); };
  if (!window.__slowSwitch) return;
  p.switchTo = (item, o) => {
    window.__out.switches.push(item.url);
    return new Promise((done) => { window.__letSwitch = () => o.commit().then(() => done({ switched: true, residualMs: 0.01, offsetS: 0 })); });
  };
}

/* the playing copy's `stream` event: a peer's MP3 128, all here unless `o` says otherwise; no offer while lanes race */
const playing = (id, o = {}) => ({
  trackId: id, jobId: 'low-job', status: 'streaming', attempt: 1, peer: 'lowpeer', format: 'MP3 128', youtube: false,
  received: SIZE, size: SIZE, ranges: [[0, 1]], speed: 0, seekAheadS: null, failedPeers: [], peersFound: 6, belowMinimum: true,
  unverified: false, upgrade: null, ...o,
});
const offer = (o = {}) => ({ peer: 'winner', path: PATH, format: 'MP3 320', status: 'streaming', received: SIZE, size: SIZE, ready: true, ...o });

async function setUp(page, request, { slowSwitch = false, desktop = false } = {}) {
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  if (desktop) {
    await page.addInitScript({ path: bridge });
    await page.addInitScript(() => {
      window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' };
      /* what main sends the mini player */
      window.__snaps = [];
      const t = window.__TAURI__, emit = t.event.emit;
      t.event.emit = (name, p) => { if (name === 'player') window.__snaps.push({ at: Date.now(), pos: p.pos, playing: p.playing }); return emit(name, p); };
    });
  }
  if (!desktop) {
    await page.addInitScript(fakeOutput);
    if (slowSwitch) await page.addInitScript(() => { window.__slowSwitch = true; });
    await page.addInitScript(recordLoads);
  }
  await openScreen(page, '/home', { clock: false });
  const [id] = await page.evaluate(() => App.data.api('/__fixture/unsaved-songs?n=1').then((r) => { App.data.absorb(r.entities); return r.items; }));
  await page.evaluate((id) => App.player.playList([id], 0), id);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.2);
  const emit = (data) => request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() }, data: { topic: 'stream', data } });
  await emit(playing(id));
  await page.waitForFunction(() => App.player.state.peer && App.player.state.peer.name === 'lowpeer');
  const dur = await page.evaluate(() => App.player.duration());
  return { id, emit, dur };
}

const out = (page) => page.evaluate(() => ({ loads: window.__out.loads.length, seeks: window.__out.seeks.slice(), switches: window.__out.switches.slice() }));
const pos = (page) => page.evaluate(() => App.player.state.pos);
const seek = (page, f) => page.evaluate((f) => App.player.seekFraction(f), f);
const upgradeState = (page) => page.evaluate(() => App.player.state.upgrade && App.player.state.upgrade.state);
/* the seeks the output was asked for that went back before `s` (a jump back, a restart from 0) */
const back = (o, s) => o.seeks.filter((x) => x < s - 1);

test('a seek while the better copy races keeps the listener\'s place', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request);
  await seek(page, 0.5);
  const target = 0.5 * dur;
  // the race runs: the playing copy's events go on, with no offer, while lanes start and give way
  for (const n of [5, 4, 3, 6, 5]) {
    await emit(playing(id, { peersFound: n }));
    await page.waitForTimeout(150);
  }
  const o = await out(page);
  expect(o.seeks, 'S3 the listener\'s one seek').toEqual([target]);
  expect(o.loads, 'S3 the song was never loaded again').toBe(1);
  expect(await pos(page), 'S3 the place held').toBeGreaterThanOrEqual(target - 0.5);
});

test('an offer the hub withdraws is never switched to', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request);
  // a lane that sent a little, then was cancelled: its offer comes and goes
  await emit(playing(id, { upgrade: offer({ peer: 'loser', status: 'buffering', received: SIZE / 10, ready: false }) }));
  await page.waitForTimeout(300);
  await emit(playing(id, { upgrade: null }));
  await page.waitForTimeout(300);
  await seek(page, 0.3);
  const target = 0.3 * dur;
  await page.waitForTimeout(300);
  expect((await out(page)).switches, 'S4 nothing switched to while no copy is ready').toEqual([]);
  // the winner, ready
  await emit(playing(id, { upgrade: offer() }));
  await expect.poll(() => upgradeState(page), { timeout: 15_000 }).toBe('switched');
  const o = await out(page);
  expect(o.switches.length, 'S4 one switch').toBe(1);
  expect(o.switches[0], 'S4 to the winner').toContain('peer=winner');
  expect(o.switches.some((u) => u.includes('loser')), 'S4 the cancelled copy was asked for').toBe(false);
  expect(back(o, target), 'S4 no jump back').toEqual([]);
  expect(o.loads, 'S4 no restart').toBe(1);
  expect(await pos(page)).toBeGreaterThanOrEqual(target - 0.5);
});

test('a seek during the switch keeps the listener\'s place', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request, { slowSwitch: true });
  await emit(playing(id, { upgrade: offer() }));
  await expect.poll(() => upgradeState(page), { timeout: 15_000 }).toBe('switching');
  await seek(page, 0.6);
  const target = 0.6 * dur;
  await page.evaluate(() => window.__letSwitch());
  await expect.poll(() => upgradeState(page), { timeout: 15_000 }).toBe('switched');
  await page.waitForTimeout(500);
  const o = await out(page);
  expect(o.switches.length, 'S5 one switch').toBe(1);
  expect(back(o, target), 'S5 no jump back').toEqual([]);
  expect(o.loads, 'S5 no restart').toBe(1);
  expect(await pos(page), 'S5 the place held through the switch').toBeGreaterThanOrEqual(target - 0.5);
});

test('after the switch a seek anywhere in the new copy lands', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request);
  // the winner, ready by its speed with 40% here: the switch answers with the new job's 40%
  await emit(playing(id, { upgrade: offer({ received: 0.4 * SIZE, speed: 400_000 }) }));
  await expect.poll(() => upgradeState(page), { timeout: 15_000 }).toBe('switched');
  // where it arrived: at once
  await seek(page, 0.3);
  expect((await out(page)).seeks.at(-1), 'S6 a seek into what arrived lands at once').toBe(0.3 * dur);
  // past it: kept, the song playing on, until the new copy's bytes are there
  await seek(page, 0.8);
  const target = 0.8 * dur;
  expect(await page.evaluate(() => App.player.state.seekWait), 'S6 a seek past what arrived waits').not.toBeNull();
  const switched = { ...playing(id), jobId: 'low-job-switched', replaces: 'low-job', peer: 'winner', format: 'MP3 320', belowMinimum: false };
  await emit({ ...switched, received: 0.6 * SIZE, ranges: [[0, 0.6]] });
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => App.player.state.seekWait), 'S6 still waiting at 60%').not.toBeNull();
  await emit({ ...switched, received: 0.85 * SIZE, ranges: [[0, 0.85]] });
  await expect.poll(() => page.evaluate(() => App.player.state.seekWait), { timeout: 5_000 }).toBeNull();
  const o = await out(page);
  expect(o.seeks.at(-1), 'S6 the kept seek landed').toBe(target);
  expect(o.loads, 'S6 no restart').toBe(1);
  expect(await pos(page)).toBeGreaterThanOrEqual(target - 0.5);
});

test('a seek past what arrived, while the race runs, is kept and lands', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request);
  await emit(playing(id, { received: 0.2 * SIZE, ranges: [[0, 0.2]] }));
  await page.waitForTimeout(300);
  await seek(page, 0.5);
  const target = 0.5 * dur;
  expect(await page.evaluate(() => App.player.state.seekWait), 'S7 kept while its bytes come').not.toBeNull();
  // the race runs beside it: no offer, the playing copy arriving
  await emit(playing(id, { received: 0.35 * SIZE, ranges: [[0, 0.35]], peersFound: 5 }));
  await page.waitForTimeout(300);
  expect(await page.evaluate(() => App.player.state.seekWait), 'S7 still kept at 35%').not.toBeNull();
  await emit(playing(id, { received: 0.6 * SIZE, ranges: [[0, 0.6]], peersFound: 4 }));
  await expect.poll(() => page.evaluate(() => App.player.state.seekWait), { timeout: 5_000 }).toBeNull();
  const o = await out(page);
  expect(o.seeks, 'S7 the kept seek landed, once').toEqual([target]);
  expect(o.loads, 'S7 no restart').toBe(1);
});

test('after the switch the lock screen shows the song\'s place', async ({ page, request }) => {
  await page.addInitScript(() => {
    window.__ms = [];
    const ms = navigator.mediaSession;
    if (ms && ms.setPositionState) { const set = ms.setPositionState.bind(ms); ms.setPositionState = (s) => { window.__ms.push({ at: Date.now(), ...s }); set(s); }; }
  });
  const { id, emit, dur } = await setUp(page, request);
  test.skip(!(await page.evaluate(() => 'mediaSession' in navigator)), 'no Media Session in this engine');
  await emit(playing(id, { upgrade: offer() }));
  await expect.poll(() => upgradeState(page), { timeout: 15_000 }).toBe('switched');
  await seek(page, 0.7);
  await page.waitForTimeout(2500);
  const r = await page.evaluate(() => {
    const lock = window.__ms[window.__ms.length - 1];
    return { real: App.player.state.pos, lock: lock ? lock.position + (Date.now() - lock.at) / 1000 : null };
  });
  expect(r.real, 'S8 the place after the seek').toBeGreaterThanOrEqual(0.7 * dur - 0.5);
  expect(r.lock, 'S8 the lock screen was told').not.toBeNull();
  expect(Math.abs(r.lock - r.real), 'S8 the lock screen\'s place, against the song\'s').toBeLessThan(1.5);
});

test('through the race and a seek the mini player shows the song\'s place', async ({ page, request }) => {
  const { id, emit, dur } = await setUp(page, request, { desktop: true });
  await page.evaluate(() => window.__bridge.fire('mini', { action: 'hello' }));   // the mini player opens
  // lanes race (no offer), one is offered and withdrawn, the winner is offered: the page's own output switches, or
  // says why not; either way the place holds
  await emit(playing(id, { upgrade: offer({ peer: 'loser', status: 'buffering', received: SIZE / 10, ready: false }) }));
  await page.waitForTimeout(300);
  await emit(playing(id, { upgrade: null }));
  await emit(playing(id, { upgrade: offer() }));
  await page.waitForTimeout(1000);
  await seek(page, 0.7);
  await page.waitForTimeout(2500);
  const r = await page.evaluate(() => {
    const snaps = window.__snaps.filter((s) => s.playing), snap = snaps[snaps.length - 1];
    return { real: App.player.state.pos, mini: snap ? snap.pos + (Date.now() - snap.at) / 1000 : null };
  });
  expect(r.real, 'S9 the place after the seek').toBeGreaterThanOrEqual(0.7 * dur - 0.5);
  expect(r.mini, 'S9 the mini player was told').not.toBeNull();
  expect(Math.abs(r.mini - r.real), 'S9 the mini player\'s place, against the song\'s').toBeLessThan(1.5);
});
