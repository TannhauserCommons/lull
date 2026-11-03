// Code that belongs to one screen or panel loads on its first use (docs/context/web.md, Code loads where it is used):
// a browser starts with the files start-files.json lists, each with its reason, and nothing else. What loads on use, and
// what that must not change:
// - Classic mode (webamp-session.js, skins.js, classic.js, classic.css): a .wsz dropped anywhere becomes the Classic
//   skin, Classic opens styled, leaving it returns to the page it was opened from. The desktop app loads it, and its
//   bridge (desktop.js, desktop.css), at start: its crash recovery hands the queue back through classic.js.
// - Stats (stats.js, stats.css): styled on its first visit.
// - The hub's own settings (hub.js, hub.css): an unpaired device lands on the pairing gate at once, first-run setup and
//   Settings > Library, Sources and Devices open on the first click, library.source events reach what is open before
//   hub.js has loaded, and the Connect picker (its styles are the start's) is styled without it.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

const START = JSON.parse(readFileSync(new URL('./start-files.json', import.meta.url), 'utf8')).files;

const loaded = (page) =>
  page.evaluate(() => ({
    classic: !!window.LullWebamp && !!App.classicSettings,
    desktop: !!window.LullDesktop,
    hub: !!App.hubKit,
    scripts: [...document.scripts].map((s) => s.getAttribute('src')).filter((s) => /classic|skins|webamp-session|desktop\.js|hub\.js|stats\.js/.test(s || '')),
    styles: [...document.styleSheets].flatMap((sh) => [...sh.cssRules].filter((r) => r.href).map((r) => r.href)).filter((h) => /classic|desktop|hub|stats/.test(h)),
  }));

/** This test's own events on the fixture hub (server.mjs, X-Fixture-Client): its page alone hears what it emits. */
async function own(page, request) {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  return { emit: (topic, data) => request.post('/__fixture/emit', { data: { topic, data }, headers }) };
}

test.describe('in a browser', () => {
  test('starts with the files start-files.json lists, and nothing else', async ({ page }) => {
    await openScreen(page, '/home', { clock: false });   // the fixed clock replaces performance, and its entries
    const files = await page.evaluate(() => performance.getEntriesByType('resource').map((r) => new URL(r.name))
      .filter((u) => u.origin === location.origin && /\.(js|css)$/.test(u.pathname))
      .map((u) => u.pathname.slice(1).replace(/^designs\/lull(\/|\.js)/, 'designs/<design>$1')));
    const listed = Object.keys(START);
    expect(files.filter((f) => !listed.includes(f)).sort(),
      'loaded at start without a reason: load it on its first use (docs/context/web.md), or list why the start needs it').toEqual([]);
    expect(listed.filter((f) => !files.includes(f)).sort(), 'listed, but no longer loaded at start').toEqual([]);
    expect(await loaded(page)).toEqual({ classic: false, desktop: false, hub: false, scripts: [], styles: [] });
  });

  test('a .wsz dropped anywhere becomes the Classic skin', async ({ page }) => {
    await openScreen(page, '/home');
    await page.evaluate(() => {
      const dt = new DataTransfer();
      dt.items.add(new File(['PK\x03\x04 a skin'], 'Night Owl.wsz', { type: 'application/zip' }));
      document.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
    });
    await page.waitForFunction(() => App.classicSkin && App.classicSkin.get() && App.classicSkin.get().name === 'Night Owl');
  });

  test('leaving Classic returns to the page it was opened from', async ({ page }) => {
    await openScreen(page, `/album/${ids.album}`);
    await page.keyboard.press('w');
    // its stage styled from the first frame: classic.css comes in with the code
    expect(await page.locator('.cls-root').first().evaluate((el) => getComputedStyle(el).position)).toBe('fixed');
    await page.locator('[data-act="cls-exit"]').first().click();
    await page.waitForFunction((album) => location.hash === `#/album/${album}`, ids.album);
  });

  test('Stats loads on its first visit, styled', async ({ page }) => {
    await openScreen(page, '/home');
    await page.evaluate(() => { location.hash = '#/stats'; });
    await expect(page.locator('.sys-tiles').first()).toHaveCSS('display', 'grid');
  });

  test('Settings > Appearance shows Classic mode\'s group', async ({ page }) => {
    await openScreen(page, '/settings/appearance');
    await expect(page.locator('#cls-browser, [data-skin-file]').first()).toBeAttached();
  });

  test('Settings > Library, Sources and Devices open on the first click', async ({ browser }) => {
    const shows = { library: '#hub-lib', sources: '#hub-slsk-status', devices: '[data-act="hub-pair"]' };
    for (const [section, part] of Object.entries(shows)) {
      const context = await browser.newContext({ viewport: { width: 1440, height: 900 } }), page = await context.newPage();
      await openScreen(page, '/settings/playback');
      expect((await loaded(page)).hub, `hub.js before opening ${section}`).toBe(false);
      await page.locator(`.sys-nav a[href="#/settings/${section}"]`).click();
      await expect(page.locator(part).first(), `Settings > ${section}`).toBeVisible();
      expect((await loaded(page)).styles.some((h) => /hub\.css/.test(h)), 'hub.css came with it').toBe(true);
      expect(await page.locator('.toast').count(), 'no error').toBe(0);
      await context.close();
    }
  });

  test('first-run setup opens with its steps', async ({ page }) => {
    await openScreen(page, '/setup');
    await expect(page.locator('.setup #hub-lib')).toBeVisible();
    await expect(page.locator('.sys-step')).toHaveCount(3);
  });

  test('an unpaired device lands on the pairing gate at once', async ({ page }) => {
    // the hub answers 401 to everything but the gate's own read, as it does for a device it doesn't know
    await page.route('**/api/**', (r) => new URL(r.request().url()).pathname === '/api/setup'
      ? r.fulfill({ json: { claimed: true, steps: { library: true, soulseek: true, lastfm: false } } })
      : r.fulfill({ status: 401, json: { error: 'unauthorized', message: 'Pair this device first.' } }));
    await page.addInitScript(() => {
      // what the gate shows, in order: its placeholder, then the form; never anything else between them
      window.__gate = [];
      new MutationObserver(() => {
        const g = document.getElementById('setup-gate'); if (!g) return;
        const now = g.querySelector('form') ? 'form' : g.querySelector('.skel, [aria-busy="true"]') ? 'placeholder' : 'other: ' + g.textContent.trim().slice(0, 60);
        if (window.__gate[window.__gate.length - 1] !== now) window.__gate.push(now);
      }).observe(document, { subtree: true, childList: true });
    });
    await page.goto('/#/home');
    await expect(page.locator('#setup-name')).toBeVisible();
    expect(await page.inputValue('#setup-name'), 'the device name hub.js suggests').not.toBe('');
    expect(await page.evaluate(() => window.__gate)).toEqual(['placeholder', 'form']);
    // the form's own focus holds: the dialog's first focus (30 ms in) doesn't take it back, however soon the form came
    await expect(page.locator('#setup-code')).toBeFocused();
  });

  test('library.source events reach what is open before hub.js loads', async ({ page, request }) => {
    const hub = await own(page, request);
    await openScreen(page, '/home');
    const asked = [];
    page.on('request', (r) => { const p = new URL(r.url()).pathname; if (p === '/api/library/source' || p === '/api/setup') asked.push(p); });
    await hub.emit('library.source', { mode: 'jellyfin', state: 'failed', detail: 'Jellyfin stopped answering (pinned by the gate)' });
    await expect.poll(() => [...new Set(asked)].sort(), 'reads fetched again after the switch').toEqual(['/api/library/source', '/api/setup']);
    expect((await loaded(page)).hub, 'hub.js not needed for that').toBe(false);
    // and the switch's last word shows once Settings > Library opens
    await page.evaluate(() => { location.hash = '#/settings/library'; });
    await expect(page.locator('.hub-lib__progress')).toContainText('Jellyfin stopped answering (pinned by the gate)');
  });

  test('the Connect picker is styled without hub.js', async ({ page }) => {
    await openScreen(page, '/home');
    // measured as it opens: its "Manage devices" link takes the focus, and that fetches hub.js ahead (the way in)
    const opened = await page.evaluate(() => {
      App.connect.open(document.querySelector('.pbar') || document.body);
      const pop = document.querySelector('.cx-pop');
      const list = pop && pop.querySelector('.cx-list');
      return { width: pop && pop.getBoundingClientRect().width, listIndent: list && getComputedStyle(list).paddingLeft,
        hubCss: [...document.styleSheets].some((sh) => [...sh.cssRules].some((r) => r.href && /hub\.css/.test(r.href) && r.styleSheet && r.styleSheet.cssRules.length)) };
    });
    expect(opened.hubCss, 'hub.css not in yet').toBe(false);
    expect(opened.width, 'its own width, from system.css').toBeLessThanOrEqual(380);
    expect(opened.listIndent, 'its device list, from system.css (a bare list is indented)').toBe('0px');
  });
});

test.describe('in the desktop app', () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
  });

  test('loads Classic mode and the bridge with the page, and hears Webamp\'s window from the start', async ({ page }) => {
    await openScreen(page, '/home');
    const l = await loaded(page);
    expect(l.classic && l.desktop).toBe(true);
    expect(l.styles.length, 'classic.css and desktop.css').toBe(2);
    // what the window reports after a crash reaches classic.js without Classic having been opened on this page
    await page.evaluate(() => window.__bridge.fire('classic', { type: 'failed', message: 'pinned by the gate' }));
    await expect(page.locator('.toast', { hasText: 'pinned by the gate' })).toBeVisible();
  });
});
