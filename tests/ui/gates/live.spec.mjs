// The live model (ADR-0101, docs/themes.md section 14) in a real page: a listener's theme that names clock values gets
// them in its slot's words, on its live hosts and on :root, and they turn at the minute's edge, not while the page is
// hidden; a built-in design, which names none, loads none of it and costs what it did. On a fixture hub of the gate's
// own, which lists a theme's live values as the hub does (crates/themes live.rs).

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { designs, ids, openScreen, ownHub, quiet, settle, traced } from './lib.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const { budgets: B } = JSON.parse(readFileSync(join(here, 'perf-budgets.json'), 'utf8'));
const MANIFEST = { id: 'clocked', name: 'Clocked', api: 1, extends: 'lull', shell: 'rail', swatches: ['#141516', '#d6d3c9', '#ffa53d', '#ffb24a'], modes: ['day', 'night'] };
const CSS = `.ck-hand { display: inline-block; width: 2px; height: 10px; background: currentColor; --clock-minute: inherit; rotate: calc(var(--clock-minute) * 6deg); }
:root[data-clock-daypart="night"] .ck-time { text-decoration: underline; }`;
const BRAND = '<span class="wordmark ck"><span class="ck-time" data-bind="clock.time"></span> <b data-bind="clock.hour"></b>:<b data-bind="clock.minute"></b><i class="ck-hand" data-live></i></span>';

// a theme that shows the song, what comes next, where it comes from and the repeat mode, outside the player
const SONG_CSS = `.sg-pos { --song-pos: inherit; scale: var(--song-pos) 1; }
:root[data-player-repeat="one"] .sg-title { text-decoration: underline; }`;
const SONG_BRAND = '<span class="wordmark sg"><b class="sg-title" data-bind="song.title"></b> <b class="sg-next" data-bind="next.title"></b> <b class="sg-src" data-bind="song.source"></b> <b class="sg-el" data-bind="song.elapsed"></b><i class="sg-pos" data-live></i></span>';

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-live-'));
  mkdirSync(join(dir, 'clocked', 'slots'), { recursive: true });
  writeFileSync(join(dir, 'clocked', 'theme.json'), JSON.stringify(MANIFEST));
  writeFileSync(join(dir, 'clocked', 'theme.css'), CSS);
  writeFileSync(join(dir, 'clocked', 'slots', 'brand.html'), BRAND);
  mkdirSync(join(dir, 'songful', 'slots'), { recursive: true });
  writeFileSync(join(dir, 'songful', 'theme.json'), JSON.stringify({ ...MANIFEST, id: 'songful', name: 'Songful' }));
  writeFileSync(join(dir, 'songful', 'theme.css'), SONG_CSS);
  writeFileSync(join(dir, 'songful', 'slots', 'brand.html'), SONG_BRAND);
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

async function context(browser) {
  return browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 }, timezoneId: 'UTC', locale: 'en-GB' });
}
const shown = (page) => page.evaluate(() => {
  const ck = document.querySelector('.ck');
  return { hour: ck.querySelectorAll('b')[0].textContent, minute: ck.querySelectorAll('b')[1].textContent,
    time: ck.querySelector('.ck-time').textContent, hand: ck.querySelector('.ck-hand').style.getPropertyValue('--clock-minute'),
    daypart: document.documentElement.getAttribute('data-clock-daypart') };
});

test('the hub lists the clock values the theme names', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  const served = await (await page.request.get('/api/themes/clocked/theme.json')).json();
  expect(served.live).toEqual(['clock.daypart', 'clock.hour', 'clock.minute', 'clock.time']);
  const song = await (await page.request.get('/api/themes/songful/theme.json')).json();
  expect(song.live).toEqual(['next.title', 'player.repeat', 'song.elapsed', 'song.source', 'song.title']);
  await ctx.close();
});

test('a theme that names the clock shows it, and it turns at the minute\'s edge', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await page.clock.install({ time: new Date('2026-11-08T21:59:58Z') });
  await openScreen(page, '/home', { design: 'clocked', clock: false });
  await expect(page.locator('.ck b').first()).toHaveText('21');
  expect(await shown(page)).toEqual({ hour: '21', minute: '59', time: '21:59', hand: '59', daypart: 'evening' });
  // lull's own hosts carry it too: every region, drawn before the theme's script had loaded
  expect(await page.evaluate(() => [...document.querySelectorAll('[data-region="header"], [data-region="playerbar"]')]
    .map((el) => el.style.getPropertyValue('--clock-minute')))).toEqual(['59', '59']);
  await page.clock.runFor(3000);
  await expect(page.locator('.ck b').first()).toHaveText('22');
  expect(await shown(page)).toEqual({ hour: '22', minute: '00', time: '22:00', hand: '0', daypart: 'night' });
  await ctx.close();
});

test('nothing turns while the page is hidden; it catches up once shown', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await page.clock.install({ time: new Date('2026-11-08T10:15:30Z') });
  await openScreen(page, '/home', { design: 'clocked', clock: false });
  await expect(page.locator('.ck b').nth(1)).toHaveText('15');
  const visibility = (hidden) => page.evaluate((h) => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => h });
    document.dispatchEvent(new Event('visibilitychange'));
  }, hidden);
  await visibility(true);
  await page.clock.runFor(5 * 60000);
  expect((await shown(page)).minute, 'hidden: no minute published').toBe('15');
  await visibility(false);
  await expect(page.locator('.ck b').nth(1)).toHaveText('20');
  await ctx.close();
});

/* what the minute's edge writes, alone: a new minute and hour published to every host and binding on Home (the page's
   other timers, which a fast-forwarded clock would run in the same window, aren't the clock's cost) */
test('the minute\'s edge restyles only the hosts and the words, and forces no layout', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, '/home', { design: 'clocked' });
  await quiet(browser, page);
  const s = await traced(browser, page, async () => {
    await page.evaluate(() => { console.timeStamp('gate-start'); Live.feed({ 'clock.minute': 7, 'clock.hour': 3 }); });
    await page.waitForTimeout(300);
    await page.evaluate(() => console.timeStamp('gate-end'));
  });
  expect(await page.locator('.ck b').nth(1).textContent()).toBe('07');
  expect(s.marked[0] ?? 0, 'elements restyled by one minute\'s publish').toBeLessThanOrEqual(B.fanOutElements);
  expect(s.forcedLayouts, 'layouts forced by it').toBe(0);
  await ctx.close();
});

test('a theme that names the song shows it, what comes next and the repeat mode, outside the player, and they follow the player', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, `/album/${ids.album}`, { design: 'songful' });
  const queue = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playTrack(rows[0], rows);
    return rows.slice(0, 3).map((id) => App.data.track(id).title);
  });
  const sg = () => page.evaluate(() => ({ title: document.querySelector('.sg-title').textContent, next: document.querySelector('.sg-next').textContent,
    source: document.querySelector('.sg-src').textContent, sourceRoot: document.documentElement.getAttribute('data-song-source'),
    repeat: document.documentElement.getAttribute('data-player-repeat') }));
  await expect(page.locator('.sg-title').first()).toHaveText(queue[0]);
  const first = await sg();
  expect(first.next).toBe(queue[1]);
  expect(first.source).not.toBe('none');
  expect(first.sourceRoot).toBe(first.source);
  expect(first.repeat).toBe('off');
  await page.evaluate(() => App.player.next());
  await expect(page.locator('.sg-title').first()).toHaveText(queue[1]);
  expect((await sg()).next).toBe(queue[2]);
  await page.evaluate(() => { App.player.cycleRepeat(); App.player.cycleRepeat(); });
  expect((await sg()).repeat).toBe('one');
  await ctx.close();
});

test('a theme that names the song\'s place and its seconds keeps the player\'s tick within its budget', async ({ browser }) => {
  const ctx = await context(browser), page = await ctx.newPage();
  await openScreen(page, `/album/${ids.album}`, { design: 'songful' });
  await page.evaluate(() => { const r = document.querySelector('#view .trow[data-track-id]'); App.player.playTrack(r.getAttribute('data-track-id'), [r.getAttribute('data-track-id')]); });
  await page.waitForFunction(() => App.player.state.playing);
  await page.evaluate(() => { location.hash = '#/home'; });
  await settle(page);
  const s = await traced(browser, page, () => page.waitForTimeout(3000));
  expect(s.restyled[0] ?? 0, 'most elements restyled in one pass while playing').toBeLessThanOrEqual(B.tickRestyleElements);
  expect(s.forcedLayouts, 'layouts forced while playing').toBe(0);
  await ctx.close();
});

for (const design of designs) {
  test(`${design}, which names no live value, loads none of it`, async ({ browser }) => {
    const ctx = await context(browser), page = await ctx.newPage();
    const asked = [];
    page.on('request', (r) => { if (/\/live\.js(\?|$)/.test(r.url())) asked.push(r.url()); });
    await openScreen(page, '/home', { design });
    expect(asked, 'live.js fetched').toEqual([]);
    expect(await page.evaluate(() => [typeof window.Live, document.documentElement.getAttributeNames().filter((a) => a.startsWith('data-clock-'))]))
      .toEqual(['undefined', []]);
    await ctx.close();
  });
}
