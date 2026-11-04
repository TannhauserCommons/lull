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
async function openWrapper(page, hash, pkg) {
  await page.addInitScript({ path: bridge });
  await page.addInitScript((p) => { window.__bridge.answers.theme_package = (args) => (p && args.id === p.manifest.id ? p : null); }, pkg);
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
