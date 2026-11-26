// The theme API's promise, kept in a real page (docs/themes.md sections 3, 9 and 11, ADR-0004): a theme written for
// api 1 keeps working, release after release, over every built-in design. fixtures/themes/theme-api-v1 is such a
// theme, frozen: each rule names a stable hook on the screen it is checked on (`.app[data-screen="album"] .lib-hero`)
// (and, as section 5 writes them, by id, on the page's root and by the values they hold: `#pbar[data-playing="true"]`)
// and sets a value of its own, a custom property, and for each region a colour the designs set themselves. Over each
// design (the theme extends it, in its shell) the gate opens those screens with a song playing and asks of every rule
// that applies there that it matches an element and that its values land: a hook renamed, or no longer drawn where it
// was, fails here by its rule. And so that the theme grows with the API, a stable hook (web/designs/hooks.js) that
// every design draws on these screens and that no rule names fails too, with the rule to add. On a fixture hub of the
// gate's own.

import { expect, test } from '@playwright/test';
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { designs, ids, openScreen, ownHub, settle } from './lib.mjs';

const web = (path) => fileURLToPath(new URL(`../../../web/${path}`, import.meta.url));
const fixture = fileURLToPath(new URL('../fixtures/themes/theme-api-v1', import.meta.url));
const css = readFileSync(join(fixture, 'theme.css'), 'utf8');

/* the stable hooks, as lull ships them (classes and attributes by their names; a wildcard stands for no one name) */
const STABLE = JSON.parse(/var HOOKS = (\{.*\});/.exec(readFileSync(web('designs/hooks.js'), 'utf8'))[1]).stable;
/* hooks the screens hold for a moment or as the song goes on, or by how wide the platform's fonts set the words: drawn
   or not by when the gate looks, so never asked of the theme */
const VARIES = new Set(['.is-power-on', '.is-played', '.is-done', '.progress', 'data-fit-out', 'data-tip-cut']);

// each page is where the theme's rules for one screen are checked; the phone's has the mini player. Each is drawn with
// the song already playing (started on Playlists, a screen the gate doesn't check): a design's slot for a playing
// song alone (Glasshouse's homeLead.playing) draws only then
const DESKTOP = ['/home', '/now', `/album/${ids.album}`, `/playlist/${ids.playlist}`, `/artist/${ids.artist}`, '/search?q=velvet', '/library', '/settings/appearance', '/stats'];
const PHONE = ['/home'];

let hub, dir;
test.beforeAll(async () => {
  dir = mkdtempSync(join(tmpdir(), 'lull-theme-api-'));
  const manifest = JSON.parse(readFileSync(join(fixture, 'theme.json'), 'utf8'));
  for (const design of designs) {
    const id = `${manifest.id}-${design}`, { shell } = JSON.parse(readFileSync(web(`designs/${design}/theme.json`), 'utf8'));
    cpSync(fixture, join(dir, id), { recursive: true });
    writeFileSync(join(dir, id, 'theme.json'), JSON.stringify({ ...manifest, id, extends: design, shell }));
  }
  hub = await ownHub({ FIXTURE_THEMES: dir });
});
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

/* the theme's rules as a browser reads them, each with the .app compound that says where it is checked: a rule without
   one (a part outside the app: the sky, the skip link) is checked on every screen */
const SCREEN = /\.app(?:\[[^\]]+\]|:not\(\[[^\]]+\]\))+/;
let RULES;
async function readRules(browser) {
  const page = await browser.newPage();
  const rules = await page.evaluate((text) => {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(text);
    return [...sheet.cssRules].map((r) => ({ selector: r.selectorText, decls: [...r.style].map((p) => [p, r.style.getPropertyValue(p).trim()]) }));
  }, css);
  await page.close();
  return rules.map((r) => ({ ...r, scope: (SCREEN.exec(r.selector) || [''])[0] }));
}

/* in the page: each rule that applies to this screen (null where it doesn't), and what is wrong with it ('' when nothing) */
const check = (page) => page.evaluate((rules) => {
  const app = document.getElementById('app');
  return rules.map((r) => {
    if (r.scope && !app.matches(r.scope)) return null;
    const els = [...document.querySelectorAll(r.selector)];
    if (!els.length) return 'matches nothing';
    return r.decls.filter(([p, v]) => !els.some((e) => getComputedStyle(e).getPropertyValue(p).trim() === v))
      .map(([p, v]) => `${p} is ${JSON.stringify(getComputedStyle(els[0]).getPropertyValue(p).trim())}, not ${v}`).join('; ');
  });
}, RULES);
/* in the page: the stable hooks on it, by the names they are listed by */
const drawn = (page) => page.evaluate((stable) => {
  const out = new Set();
  for (const el of document.querySelectorAll('*')) {
    for (const c of el.classList) if (stable.classes.includes(c)) out.add('.' + c);
    for (const a of el.getAttributeNames()) if (stable.attrs.includes(a)) out.add(a);
  }
  return [...out];
}, STABLE);

/* the theme over one design: what failed, each saying where, and every stable hook the design drew, with where first */
async function over(browser, design) {
  const id = `theme-api-v1-${design}`, failed = [], seen = new Map(), checked = new Set();
  for (const [routes, use] of [[DESKTOP, { viewport: { width: 1440, height: 900 } }], [PHONE, { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true }]]) {
    const context = await browser.newContext({ baseURL: hub.url, ...use, serviceWorkers: 'block' });
    const page = await context.newPage();
    await openScreen(page, '/playlists', { design: id });
    await page.waitForFunction((want) => Designs.current === want, id);
    const songs = (await (await page.request.get(`/api/albums/${ids.album}`)).json()).album.trackIds;
    await page.evaluate((s) => App.player.playTrack(s[0], s), songs);
    await page.waitForFunction(() => App.player.state.playing);
    for (const route of routes) {
      await page.evaluate((r) => { location.hash = '#' + r; }, route);
      // drawn once lull has the screen's code (Settings' comes on its first use)
      await page.waitForFunction((path) => App.current && App.current.path === path, route.split('?')[0]);
      await settle(page);
      // a value may still be on its way through a design's transition
      let said;
      for (let i = 0; i < 20; i++) {
        said = await check(page);
        if (said.every((s) => !s)) break;
        await page.waitForTimeout(250);
      }
      said.forEach((s, i) => {
        if (s === null) return;
        checked.add(i);
        if (s) failed.push(`${design}, ${route}${use.isMobile ? ' (phone)' : ''}: ${RULES[i].selector}: ${s}`);
      });
      for (const h of await drawn(page)) if (!seen.has(h)) seen.set(h, route);
    }
    await context.close();
  }
  RULES.forEach((r, i) => { if (!checked.has(i)) failed.push(`${design}: ${r.selector}: no screen the gate opens is ${r.scope}`); });
  return { failed, seen };
}

test('a theme written for api 1 still styles every stable hook it names, over every built-in design', async ({ browser }) => {
  test.setTimeout(180_000);
  RULES = await readRules(browser);
  const runs = await Promise.all(designs.map((d) => over(browser, d)));
  // (a region's colour and the hook's own value are two rules with one selector: said once)
  expect([...new Set(runs.flatMap((r) => r.failed))], 'rules of fixtures/themes/theme-api-v1 that no longer match or land').toEqual([]);
  // a stable hook every design draws here that no rule names yet: its rule to add, on a screen that draws it
  const named = new Set(RULES.flatMap((r) => [
    ...(r.selector.replace(/\[[^\]]*\]/g, '').match(/\.[a-zA-Z_][\w-]*/g) || []),
    ...(r.selector.match(/data-[a-z][a-z0-9]*(?:-[a-z0-9]+)*/g) || []),
  ]));
  const unnamed = [...runs[0].seen].filter(([h]) => runs.every((r) => r.seen.has(h)) && !named.has(h) && !VARIES.has(h));
  expect(unnamed.map(([h, route]) => `${h}, drawn on ${route}`).sort(),
    'stable hooks the screens draw that fixtures/themes/theme-api-v1 doesn\'t style yet: add a rule for each').toEqual([]);
});
