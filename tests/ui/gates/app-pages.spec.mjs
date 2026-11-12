// The desktop app's own pages (desktop/wrapper.html: the mini player, the strip, connect, unreachable) come from the
// app, not the hub, so a listener's theme reaches them only as the package the web UI in `main` hands the app
// (app/src/desktop/theme.rs). Here the bridge stands in for the app (desktop-bridge.js), and the test carries the
// package from main's page to the wrapper's as Rust does. docs/themes.md, section 5.

import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { ownHub } from './lib.mjs';

const bridge = fileURLToPath(new URL('./desktop-bridge.js', import.meta.url));
const fixture = fileURLToPath(new URL('../fixtures/themes', import.meta.url));

// The hub's themes folder, a copy the test may edit as a listener would on the hub.
let themes, hub;
test.beforeAll(async () => {
  themes = mkdtempSync(join(tmpdir(), 'lull-themes-'));
  cpSync(fixture, themes, { recursive: true });
  hub = await ownHub({ FIXTURE_THEMES: themes });
});
test.afterAll(() => { hub?.close(); rmSync(themes, { recursive: true, force: true }); });
// the listener's edits, undone after the test that made them
const restore = [];
test.afterEach(() => { while (restore.length) restore.pop()(); });

// main: the hub's web UI in the desktop app, showing the listener's theme. Resolves to the page.
async function openMain(page) {
  await page.addInitScript({ path: bridge });
  await page.addInitScript(() => localStorage.setItem('lull:prefs', JSON.stringify({ design: 'paper-moon', theme: 'night' })));
  await page.goto(`${hub.url}/#/home`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked && Designs.current === 'paper-moon');
  return page;
}
const handedOver = (page, after = 0) => page.waitForFunction((n) => (window.__bridge.calls.set_theme_package || 0) > n &&
  window.__bridge.last.set_theme_package.package, after).then(() => page.evaluate(() => window.__bridge.last.set_theme_package.package));

// a wrapper page as the app serves it: no hub beside it (every /api answers 404), the app's bridge answering
// theme_package with what main handed over
async function openWrapper(page, hash, pkg, init) {
  await page.addInitScript({ path: bridge });
  await page.addInitScript((p) => { window.__bridge.answers.theme_package = (args) => (p && args.id === p.manifest.id ? p : null); }, pkg);
  if (init) await page.addInitScript(init);
  await page.route('**/api/**', (r) => r.fulfill({ status: 404, body: 'not here' }));
  await page.goto(`${hub.url}/desktop/wrapper.html#${hash}`);
  await page.waitForFunction(() => !document.documentElement.hasAttribute('data-booting'));
}
const look = (page) => page.evaluate(() => ({
  design: Designs.current, loaded: document.documentElement.dataset.design,
  mark: getComputedStyle(document.documentElement).getPropertyValue('--moon-mark').trim(),
  brand: String(Designs.slot('brand', { where: 'desktop' }, () => '')),
}));

test('main hands the app its listener theme in one package, its assets inlined', async ({ page }) => {
  await openMain(page);
  const pkg = await handedOver(page);
  expect(pkg.manifest).toMatchObject({ id: 'paper-moon', extends: 'skydeck', shell: 'rail' });
  expect(pkg.css, 'the font and the picture travel with it').toContain('url("data:font/woff2;base64,');
  expect(pkg.css).toContain('url("data:image/svg+xml;base64,');
  expect(pkg.css, 'no url() left that only the hub can answer').not.toMatch(/url\(\s*["']?assets\//);
  expect(pkg.slots.brand).toContain('moon-brand');
  // and the pages it opens are told the design the theme extends, for while they have no package
  expect(await page.evaluate(() => window.__bridge.last.appearance)).toMatchObject({ design: 'paper-moon', base: 'skydeck' });

  // a built-in design is the app's own: nothing to hand over
  await page.evaluate(() => App.setDesign('petal'));
  await page.waitForFunction(() => 'package' in window.__bridge.last.set_theme_package && window.__bridge.last.set_theme_package.package === null);
  expect(await page.evaluate(() => window.__bridge.last.appearance.base)).toBeUndefined();
});

test('the app\'s pages show the theme main handed over, the design it extends until then, and follow its edits', async ({ page, browser }) => {
  // the mini player opens before main has handed anything over: the design the theme extends, never lull
  await openWrapper(page, 'view=mini&design=paper-moon&base=skydeck&theme=night&opaque=1', null);
  await page.waitForSelector('[data-dk-player]');
  expect(await look(page)).toMatchObject({ design: 'skydeck', loaded: 'skydeck', mark: '' });

  const main = await openMain(await browser.newPage());
  const pkg = await handedOver(main);

  // main hands it over: the theme itself, on the design it extends
  await page.evaluate((p) => { window.__bridge.answers.theme_package = (args) => (args.id === p.manifest.id ? p : null); }, pkg);
  await page.evaluate(() => window.__bridge.fire('theme-package', { id: 'paper-moon' }));
  await page.waitForFunction(() => Designs.current === 'paper-moon');
  const shown = await look(page);
  expect(shown).toMatchObject({ loaded: 'skydeck', mark: '"paper"' });
  expect(shown.brand, 'its slots, sanitized').toContain('moon-brand');
  expect(await page.evaluate(() => document.fonts.check('16px "Paper moon"') && document.fonts.load('16px "Paper moon"').then((f) => f.length)),
    'its font, from the package').toBe(1);
  await page.waitForSelector('[data-dk-player]');

  // the listener edits the theme on the hub: main reloads it and hands it over again, and the open mini player follows
  const css = join(themes, 'paper-moon', 'theme.css'), was = readFileSync(css, 'utf8');
  writeFileSync(css, ':root { --moon-mark: "edited"; }\n');
  restore.push(() => writeFileSync(css, was));
  await fetch(`${hub.url}/__fixture/emit`, { method: 'POST', headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ topic: 'theme.changed', data: { id: 'paper-moon' } }) });
  const edited = await handedOver(main, await main.evaluate(() => window.__bridge.calls.set_theme_package));
  expect(edited.css).toContain('"edited"');
  await page.evaluate((p) => { window.__bridge.answers.theme_package = (args) => (args.id === p.manifest.id ? p : null); }, edited);
  await page.evaluate(() => window.__bridge.fire('theme-package', { id: 'paper-moon' }));
  await expect.poll(() => look(page).then((l) => l.mark)).toBe('"edited"');
  await page.waitForSelector('[data-dk-player]');

  // main switches to a built-in design: the mini player follows by the appearance event alone
  await page.evaluate(() => window.__bridge.fire('appearance', { design: 'petal', theme: 'day' }));
  await expect.poll(() => look(page)).toMatchObject({ design: 'petal', loaded: 'petal', mark: '' });
  await main.close();
});

test('the app\'s pages open in the listener theme main already handed over, whatever the view', async ({ page, browser }) => {
  const main = await openMain(await browser.newPage());
  const pkg = await handedOver(main);
  await main.close();
  for (const view of ['strip', 'connect', 'offline&url=http%3A%2F%2Fhub.invalid', 'hub-error&error=pinned']) {
    const p = await browser.newPage();
    await openWrapper(p, `view=${view}&design=paper-moon&base=skydeck&theme=day&opaque=1`, pkg);
    expect(await look(p), view).toMatchObject({ design: 'paper-moon', loaded: 'skydeck', mark: '"paper"' });
    await p.close();
  }
  // and without it (the hub was down when the app started), the design the theme extends
  await openWrapper(page, 'view=offline&url=http%3A%2F%2Fhub.invalid&design=paper-moon&base=skydeck&theme=day', null);
  expect(await look(page)).toMatchObject({ design: 'skydeck', loaded: 'skydeck' });
});

// The mini player's keys (web/desktop/mini-keys.json, the one list desktop.js runs and lull-themes checks slots against):
// in every design's mini player and strip, each key a listener can click reaches what runs it, main or the window.
const KEYS = JSON.parse(readFileSync(fileURLToPath(new URL('../../../web/desktop/mini-keys.json', import.meta.url)), 'utf8'));
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
// what the page sends across, in order: 'mini' events to main, and window actions
function recordSent() {
  const t = window.__TAURI__, sent = window.__sent = [];
  const emitTo = t.event.emitTo, invoke = t.core.invoke;
  t.event.emitTo = (to, name, p) => { if (name === 'mini') sent.push({ to, ...p }); return emitTo(to, name, p); };
  t.core.invoke = (cmd, args) => { if (cmd === 'window_action') sent.push({ window: args.request.action }); return invoke(cmd, args); };
}

test('every key in the mini player and the strip reaches what runs it, in every design', async ({ browser }) => {
  for (const design of DESIGNS) {
    for (const view of ['mini', 'strip']) {
      const at = `${design} ${view}`, page = await browser.newPage();
      await openWrapper(page, `view=${view}&design=${design}&theme=day&opaque=1`, null, recordSent);
      await page.waitForFunction((d) => Designs.current === d && document.querySelector('[data-dk-player]'), design);
      await page.evaluate(() => window.__bridge.fire('player', { track: { id: 't1', title: 'Song', artist: 'Artist', format: 'FLAC', lossless: true },
        playing: false, liked: false, pos: 20, duration: 200, durationKnown: true, buffered: [[0, 1]], received: 1, shuffle: false, repeat: 'off', volume: .5 }));
      const keys = await page.$$eval('[data-dk-player] [data-dk]', (els) => els.map((el) => el.getAttribute('data-dk')));
      expect(keys.filter((k) => !(k in KEYS)), `${at}: keys nothing runs`).toEqual([]);
      expect(keys, `${at}: the transport`).toEqual(expect.arrayContaining(['previous', 'play-pause', 'next']));
      for (let i = 0; i < keys.length; i++) {
        const key = keys[i], before = await page.evaluate(() => window.__sent.length);
        await page.locator('[data-dk-player] [data-dk]').nth(i).click();
        const sent = await page.evaluate((n) => window.__sent.slice(n), before);
        const want = KEYS[key] === 'window' ? { window: key } : KEYS[key] === 'seek' ? { to: 'main', action: 'seek' } : { to: 'main', action: key };
        expect(sent, `${at}: ${key}`).toEqual([expect.objectContaining(want)]);
      }
      await page.close();
    }
  }
});

// The unreachable page when the hub's name doesn't resolve yet (Tailscale's DNS not up when the app started): the
// wrapper reconnects (app/src/reconnect.rs) and says how each attempt went; the page only shows it. It never retries by
// a timer of its own, so a hidden window or a probe that never returns can't leave it on "trying now".
test('the unreachable page shows how the wrapper reconnects, and keeps no timer of its own', async ({ page }) => {
  const DNS = 'No such host is known. (os error 11001)';
  await openWrapper(page, `view=offline&url=${encodeURIComponent('https://lull.example.ts.net')}&error=${encodeURIComponent(DNS)}&design=lull&theme=day&opaque=1`, null,
    () => { window.__bridge.answers.reconnect_status = { attempt: 1, trying: false, error: 'No such host is known. (os error 11001)', nextIn: 2 }; });
  const card = page.locator('[data-dk-offline]'), label = card.locator('.dk-retry__label');
  await expect(label).toHaveText('retrying in 2s');
  await expect(card.locator('[data-dk-attempt]')).toHaveText('1');
  await expect(card.locator('[data-dk-error]')).toHaveText(DNS);

  // the window hidden (minimised, or in the tray), past the countdown: the page asks nothing; the wrapper tries
  await page.evaluate(() => { Object.defineProperty(document, 'hidden', { value: true, configurable: true }); document.dispatchEvent(new Event('visibilitychange')); });
  await page.waitForTimeout(3500);
  expect(await page.evaluate(() => window.__bridge.calls.window_action || 0), 'no retry from the page itself').toBe(0);

  // the wrapper tries, the probe runs out of time, it says so and waits longer
  await page.evaluate(() => window.__bridge.fire('reconnect', { attempt: 1, trying: true, error: 'No such host is known. (os error 11001)', nextIn: 0 }));
  await expect(label).toHaveText('trying now');
  await page.evaluate(() => window.__bridge.fire('reconnect', { attempt: 2, trying: false, error: 'The hub didn\'t answer within 25 s', nextIn: 4 }));
  await expect(label).toHaveText('retrying in 4s');
  await expect(card.locator('[data-dk-attempt]')).toHaveText('2');
  await expect(card.locator('[data-dk-error]')).toHaveText('The hub didn\'t answer within 25 s');
  await expect(card.locator('[data-dk-retry-status]')).toHaveText('Still can\'t reach your hub. Attempt 2.');

  // Retry now asks the wrapper to try at once
  await card.getByRole('button', { name: 'Retry now' }).click();
  expect(await page.evaluate(() => window.__bridge.last.window_action)).toEqual({ request: { action: 'retry' } });
  await expect(label).toHaveText('trying now');
});

// The player's words reach the mini player windows too (docs/themes.md, The player's words): main's snapshot carries the
// state and lull's words for it, set on [data-dk-player] as data-status, data-status-text and --status-text.
test('the mini player windows carry the player\'s state and its words, for a theme to show', async ({ page }) => {
  await openWrapper(page, 'view=mini&design=lull&theme=day&opaque=1', null);
  await page.waitForSelector('[data-dk-player]');
  const snap = (status, statusText) => ({ track: { id: 't', title: 'Song', artist: 'Artist', format: 'FLAC', lossless: true }, playing: false, liked: false,
    pos: 10, duration: 100, durationKnown: true, buffered: [], received: 1, shuffle: false, repeat: 'off', volume: .5, status, statusText });
  const read = () => page.evaluate(() => {
    const el = document.querySelector('[data-dk-player]');
    return { status: el.dataset.status, text: el.dataset.statusText, prop: getComputedStyle(el).getPropertyValue('--status-text').trim() };
  });
  expect(await read()).toEqual({ status: 'idle', text: '', prop: '""' });
  for (const [status, words] of [['failed', 'YouTube couldn’t find this song'], ['buffering', 'Loading from your library'], ['streaming', 'Streaming from "mira"']]) {
    await page.evaluate((s) => window.__bridge.fire('player', s), snap(status, words));
    expect(await read(), status).toEqual({ status, text: words, prop: JSON.stringify(words) });
  }
});

// The mini player says what a button press changed (the note and the announcer); main's first snapshot is the player as
// it already was, so a window that has just opened says nothing.
test('the mini player says what changed, never the player it opened on', async ({ page }) => {
  await openWrapper(page, 'view=mini&design=lull&theme=day&opaque=1', null);
  await page.waitForSelector('[data-dk-player]');
  const snap = (volume) => ({ track: { id: 't', title: 'Song', artist: 'Artist' }, playing: true, liked: false, pos: 10, duration: 100,
    durationKnown: true, buffered: [], received: 1, shuffle: false, repeat: 'off', volume, status: 'playing', statusText: '' });
  // everything the announcer says, in order (it speaks a moment after the note shows)
  await page.evaluate(() => {
    const heard = window.__announced = [];
    new MutationObserver(() => { const t = document.getElementById('dk-announcer')?.textContent; if (t) heard.push(t); })
      .observe(document.body, { subtree: true, childList: true, characterData: true });
  });
  const notes = () => page.evaluate(() => [...document.querySelectorAll('.dk-mini__note.is-on')].map((n) => n.textContent));
  await page.evaluate((s) => window.__bridge.fire('player', s), snap(.7));
  expect(await notes(), 'the player as it was when the window opened').toEqual([]);
  await page.evaluate((s) => window.__bridge.fire('player', s), snap(.8));
  expect(await notes(), 'a volume press').toEqual(['volume 80%']);
  await expect.poll(() => page.evaluate(() => window.__announced)).toEqual(['volume 80%']);
});

// A theme's own mini player keys keep their words (docs/themes.md, section 6): on play-pause and like lull points the
// icon in a key at the player's state, gives an empty key its icon, and never writes over the key's own words.
test('a theme\'s mini player keys keep their own words; only lull\'s icon in them follows the player', async ({ page, browser }) => {
  const main = await openMain(await browser.newPage());
  const handed = await handedOver(main);
  await main.close();
  const mini = '<div class="km" data-dk-player><button class="km-like" data-dk="like" aria-label="Like">LIKE</button>' +
    '<button class="km-play" data-dk="play-pause" aria-label="{{playLabel}}"><lull-part name="icon" icon="{{playIcon}}"></lull-part> PLAY</button>' +
    '<button class="km-bare" data-dk="like" aria-label="Like"></button></div>';
  const pkg = { ...handed, manifest: { ...handed.manifest, slots: [...(handed.manifest.slots || []), 'miniPlayer.device'] },
    slots: { ...handed.slots, 'miniPlayer.device': mini } };
  await openWrapper(page, 'view=mini&design=paper-moon&base=skydeck&theme=day&opaque=1', pkg);
  await page.waitForSelector('.km[data-dk-player]');
  const snap = (playing, liked) => ({ track: { id: 't', title: 'Song', artist: 'Artist' }, playing, liked, pos: 10, duration: 100,
    durationKnown: true, buffered: [], received: 1, shuffle: false, repeat: 'off', volume: .5, status: 'playing', statusText: '' });
  const keys = () => page.evaluate(() => ({
    like: [document.querySelector('.km-like').textContent, document.querySelector('.km-like').getAttribute('aria-pressed')],
    play: [document.querySelector('.km-play').textContent.trim(), document.querySelector('.km-play use')?.getAttribute('href')],
    bare: document.querySelector('.km-bare use')?.getAttribute('href'),
  }));
  for (const [playing, liked] of [[true, true], [false, false], [true, false]]) {
    await page.evaluate((s) => window.__bridge.fire('player', s), snap(playing, liked));
    expect(await keys(), `playing ${playing}, liked ${liked}`).toEqual({
      like: ['LIKE', String(liked)], play: ['PLAY', playing ? '#i-pause' : '#i-play'], bare: liked ? '#i-heart-fill' : '#i-heart' });
  }
});

// One vocabulary of actions (ADR-0101 amended, F): a button of a theme's own with any of the page's actions, in the
// mini player, runs in the main window as on the page, with its data-* attributes.
test('a theme\'s own button with a page action in the mini player runs that action in the main window', async ({ page, browser }) => {
  const main = await openMain(await browser.newPage());
  const handed = await handedOver(main);
  const mini = '<div class="ka" data-dk-player><button class="ka-shuf" data-act="shuffle">SHUF</button>' +
    '<button class="ka-open" data-act="open-now" data-tab="queue">QUEUE</button><button data-dk="play-pause" aria-label="Play"></button></div>';
  const pkg = { ...handed, manifest: { ...handed.manifest, slots: [...(handed.manifest.slots || []), 'miniPlayer.device'] },
    slots: { ...handed.slots, 'miniPlayer.device': mini } };
  await openWrapper(page, 'view=mini&design=paper-moon&base=skydeck&theme=day&opaque=1', pkg);
  await page.waitForSelector('.ka[data-dk-player]');
  await page.evaluate(() => {
    const out = window.__sent = [];
    const t = window.__TAURI__, emitTo = t.event.emitTo;
    t.event.emitTo = (to, name, p) => { if (name === 'mini') out.push({ to, ...p }); return emitTo(to, name, p); };
  });
  await page.locator('.ka-shuf').click();
  await page.locator('.ka-open').click();
  expect(await page.evaluate(() => window.__sent)).toEqual([
    { to: 'main', action: 'act', name: 'shuffle', attrs: { 'data-act': 'shuffle' } },
    { to: 'main', action: 'act', name: 'open-now', attrs: { 'data-act': 'open-now', 'data-tab': 'queue' } },
  ]);
  // main runs it as the page would
  const was = await main.evaluate(() => App.player.state.shuffle);
  await main.evaluate(() => window.__bridge.fire('mini', { action: 'act', name: 'shuffle', attrs: { 'data-act': 'shuffle' } }));
  await expect.poll(() => main.evaluate(() => App.player.state.shuffle)).toBe(!was);
  await main.close();
});

// Only main's own page is the app's. lull's page in a frame of it (the picture lull draws of a listener's theme, Theme
// Studio's previews) gets the app's scripts too on Windows (WebView2 runs them in every frame, as addInitScript does
// here), but it is a view: it tells the app nothing and hears none of its events. As the app's page, the picture's frame
// told the app its design, and the mini player switched to the theme being drawn; it answered the mini player's keys too.
test('a theme\'s picture drawn in the app\'s window tells the app nothing and answers none of its keys', async ({ page }) => {
  await page.addInitScript({ path: bridge });
  await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
  // what a frame's page sends the app or listens for, gathered in the window's own page
  await page.addInitScript(() => {
    if (window.top === window) return;
    const t = window.__TAURI__, seen = (what) => (window.top.__fromFrames = window.top.__fromFrames || []).push(what);
    const { emit, emitTo, listen } = t.event, invoke = t.core.invoke;
    t.event.emit = (n, p) => { seen('emit ' + n); return emit(n, p); };
    t.event.emitTo = (to, n, p) => { seen('emit ' + n); return emitTo(to, n, p); };
    t.event.listen = (n, fn) => { seen('listen ' + n); return listen(n, fn); };
    t.core.invoke = (n, p) => { seen('invoke ' + n); return invoke(n, p); };
  });
  await page.addInitScript(() => localStorage.setItem('lull:prefs', JSON.stringify({ design: 'lull', theme: 'day' })));
  await page.goto(`${hub.url}/#/home`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked && Designs.current === 'lull');
  const drawn = page.waitForRequest((r) => r.method() === 'PUT' && r.url().includes('/api/theme-pictures/paper-moon/'), { timeout: 60_000 });
  await page.evaluate(() => Designs.refresh().then(() => App.needScript('screens/theme-pictures.js', 'theme pictures')).then(() => {
    const m = App.designs['paper-moon'];
    App.makeThemePictures({ id: m.id, rev: m.rev, label: m.label }, ['day']);
  }));
  await drawn;
  expect(await page.evaluate(() => window.__fromFrames || []), 'what the picture\'s frame said to the app or listened for').toEqual([]);
  expect(await page.evaluate(() => window.__bridge.last.appearance), 'the design main last told the app').toMatchObject({ design: 'lull' });
});
