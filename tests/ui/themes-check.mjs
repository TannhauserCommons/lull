// A quick look at screens in every theme, for a UI change or while building a theme: `mise run themes:check -- <route>...`.
// Each route is drawn with a song in the player, on desktop (1440×900) and phone (390×844), day and night, in the five
// built-in designs and in every theme package in the folder LULL_THEMES_DIR names (a folder of `<id>/theme.json`
// folders; none: the built-ins only). The route `mini` draws the desktop app's mini player windows instead (the device
// and the strip, at the size the app opens them, fitted to the player as the app fits them), through the stand-in
// bridge the gates use (gates/desktop-bridge.js), a song playing and the theme handed over as one package, as the app
// gets it. Each picture runs the checks the gates run (checks.mjs: a control outside its bar or the window, two on each
// other, text cut off, an essential missing; anything sticking out sideways; a clamp that doesn't clip; a squeezed
// title; console errors) and a contrast check against what is drawn behind each line of text (checks.mjs, contrast;
// only reported for the built-in designs, which some screens fall short of), and one contact sheet shows them all:
// tests/ui/themes-check/index.html. It exits 1 when a check finds something. Flags (themes-check-args.mjs):
//   --only <id>[,<id>...]  those looks only (built in or in LULL_THEMES_DIR); --mine: LULL_THEMES_DIR's and no built-in
//   --sizes WxH[,WxH...]   the desktop at each of these sizes instead (--size: one); a laptop at 125% is 1536x864
//   --mockup <file-or-url> the mockup beside each picture, at its size; {mode}, {route} and {size} in it are filled in
// On the fixture hub, with a copy of the theme folder, so nothing is written to it. A few minutes at most; the gates
// are the full matrix.

import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { chrome, contrast, overflow, squeezed, unclippedClamps } from './checks.mjs';
import { designs as BUILT_IN, ids, openScreen, ownHub, screens } from './gates/lib.mjs';
import { mockupUrl, parseArgs, USAGE, UsageError } from './themes-check-args.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const OUT = join(here, 'themes-check');
const BRIDGE = join(here, 'gates', 'desktop-bridge.js');
const PARALLEL = 4;
const PHONE = { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true };
// the mini player's windows as the app opens them (app/src/desktop/mini.rs); each then fits its height to the player
const MINI = { device: { viewport: { width: 340, height: 560 } }, strip: { viewport: { width: 540, height: 90 } } };
// Some built-in screens fall short of the contrast check's minimums (docs/themes.md, section 10): for them it is
// reported, not a failure, so a UI change isn't held by them; for a listener's theme it is a finding like the others.
const CONTRAST_REPORT_ONLY_FOR_BUILT_INS = true;

const fail = (message) => { console.error(`${message}\n${USAGE}`); process.exit(2); };
let opts;
try { opts = parseArgs(process.argv.slice(2)); } catch (e) { if (e instanceof UsageError) fail(e.message); throw e; }

const desktops = opts.sizes || [{ width: 1440, height: 900 }];
const PAGE_SIZES = Object.fromEntries([
  ...desktops.map((v) => [desktops.length > 1 ? `desktop-${v.width}x${v.height}` : 'desktop', { viewport: v }]),
  ['phone', PHONE],
]);
const slugOf = (path) => path.replace(/[^\w]+/g, '-').replace(/^-|-$/g, '') || 'home';
// name: a key screen's, `mini`, or the path's slug (the mockup's {route}); path: what the web UI opens
const routes = opts.routes.map((r) => {
  if (r === 'mini') return { name: 'mini', mini: true, sizes: MINI };
  const path = screens[r] || (r.startsWith('/') ? r : '/' + r);
  return { name: screens[r] ? r : slugOf(path), path, sizes: PAGE_SIZES };
});

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
const slug = (j) => `${j.route.mini ? 'mini' : slugOf(j.route.path)}--${j.design}--${j.size}--${j.mode}`;

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const started = Date.now();
const hub = await ownHub({ FIXTURE_THEMES: copy });
const browser = await chromium.launch();

/* the web UI in a look and mode, with a song in the player: the fixture album's first */
async function playFirst(page, look, errors) {
  await openScreen(page, `/album/${ids.album}`, look);
  await page.evaluate(() => { const row = document.querySelector('#view .trow[data-track-id]'); return row && App.player.playTrack(row.getAttribute('data-track-id'), [row.getAttribute('data-track-id')]); });
  await page.waitForFunction(() => App.player.current(), null, { timeout: 5000 }).catch(() => errors.push('no song started'));
}

/* what the checks find in a picture as it stands, the picture, and the contrast against what is drawn */
async function judge(page, job, errors, { seeThrough = false } = {}) {
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  const found = [...(await chrome(page)), await overflow(page), await unclippedClamps(page), await squeezed(page)].filter(Boolean).map(String);
  const file = slug(job) + (seeThrough ? '.png' : '.jpg');
  // a see-through window keeps its transparency, so the sheet shows what the desktop would show through
  await page.screenshot(seeThrough ? { path: join(OUT, file), omitBackground: true } : { path: join(OUT, file), type: 'jpeg', quality: 72 });
  const problems = [...found, ...errors], notes = [];
  const low = await contrast(page, { seeThrough });
  if (low) (job.builtIn && CONTRAST_REPORT_ONLY_FOR_BUILT_INS ? notes : problems).push('contrast ' + low);
  return { ...job, file, viewport: page.viewportSize(), problems, notes };
}

/* one picture of a web UI route: the route in that design, size and mode, with a song in the player */
async function shootPage(job) {
  const context = await browser.newContext({ baseURL: hub.url, reducedMotion: 'reduce', ...job.route.sizes[job.size] });
  const page = await context.newPage();
  const errors = listen(page);
  try {
    await playFirst(page, { design: job.design, theme: job.mode }, errors);
    await page.evaluate((r) => { location.hash = '#' + r; }, job.route.path);
    await page.waitForTimeout(400);
    return await judge(page, job, errors);
  } catch (e) {
    return { ...job, problems: [...errors, 'could not draw it: ' + e.message.split('\n')[0]], notes: [] };
  } finally { await context.close(); }
}

/* main, as the app runs the web UI, for each look once: the song it plays, as the snapshot and cover it sends the mini
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
      return { snap: l.player, art: (l['player-art'] || {}).art || null, pkg: (l.set_theme_package || {}).package || null, base: (l.appearance || {}).base || null };
    });
  } finally { await context.close(); }
}

/* one picture of a mini player window: the app's page (desktop/wrapper.html) with no hub beside it, the look main handed
   over, a song playing a third of the way in */
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
    const snap = { ...main.snap, playing: true, pos: Math.floor((main.snap.duration || 0) / 3) };
    await page.evaluate(({ s, art }) => { window.__bridge.fire('player', s); window.__bridge.fire('player-art', { art }); }, { s: snap, art: main.art });
    // the window as it stays: the first snapshot shows a note of what differs from the page's defaults (the volume, as
    // if a key had changed it) for a moment, as in the app
    await page.waitForFunction(() => !document.querySelector('.dk-mini__note.is-on'), null, { timeout: 4000 });
    await page.evaluate(() => document.fonts.ready);
    // the window fits its height to the player (desktop.js, fitWindow), as the app's does
    const fit = await page.evaluate(() => window.__bridge.last.setSize);
    if (fit && fit.height) await page.setViewportSize({ width: MINI[job.size].viewport.width, height: Math.ceil(fit.height) });
    return await judge(page, job, errors, { seeThrough: true });
  } catch (e) {
    return { ...job, problems: [...errors, 'could not draw it: ' + e.message.split('\n')[0]], notes: [] };
  } finally { await context.close(); }
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
  const alt = `${r.design}, ${r.size}, ${r.mode}`, see = r.route.mini;
  const theme = r.file ? picture(r.file, alt, see) : '';
  const shown = !r.mockup ? theme : `<div class="pair"><figure>${theme}<figcaption>theme</figcaption></figure><figure>${r.mockup.error
    ? `<p class="bad">could not open the mockup: ${esc(r.mockup.error)}</p>` : picture(r.mockup.file, 'mockup, ' + alt)}<figcaption>mockup</figcaption></figure></div>`;
  const list = (cls, items) => (items.length ? `<ul class="${cls}">${items.map((p) => `<li>${esc(p)}</li>`).join('')}</ul>` : '');
  return `<td class="${r.problems.length ? 'bad' : ''}">${shown}${list('found', r.problems)}${list('noted', r.notes)}</td>`;
};
const table = (route) => {
  const cols = Object.keys(route.sizes).flatMap((size) => ['day', 'night'].map((mode) => ({ size, mode })));
  const title = route.mini ? 'mini: the desktop app\'s mini player, device and strip' : route.path;
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
<p>${esc(sizesSaid)}${routes.some((r) => r.mini) ? '; the mini player at the width the app opens it, its height fitted to the player, over a checkerboard where the desktop shows through' : ''}. ${results.length} pictures, ${failed.length ? `<strong style="color:var(--bad)">${failed.length} with something to look at</strong>` : 'nothing found by the checks'}; ${Math.round((Date.now() - started) / 1000)} s. ${opts.mine ? 'The themes in LULL_THEMES_DIR' : opts.only ? 'The looks named' : 'Built-in designs, then the themes in LULL_THEMES_DIR'}, with a song in the player.${noted.length ? ` <span style="color:var(--note)">Contrast on a built-in design is reported in this colour, not counted.</span>` : ''}${opts.mockup ? ` The mockup, ${esc(opts.mockup)}, beside each picture at its size.` : ''}</p>
${routes.map(table).join('\n')}
`);

const where = (r) => `${r.route.mini ? 'mini' : r.route.path} ${r.design} ${r.size} ${r.mode}`;
console.log(`${results.length} pictures in ${Math.round((Date.now() - started) / 1000)} s; ${failed.length ? failed.length + ' with something to look at' : 'nothing found'}`);
for (const r of failed) console.log(`  ${where(r)}: ${r.problems.join('; ')}`);
if (noted.length) console.log(`reported, not counted (contrast on a built-in design):\n${noted.map((r) => `  ${where(r)}: ${r.notes.join('; ')}`).join('\n')}`);
const unopened = [...shots.values()].filter((s) => s.error);
if (unopened.length) console.log(`mockup not drawn:\n${unopened.map((s) => `  ${s.url}: ${s.error}`).join('\n')}`);
console.log('contact sheet: ' + pathToFileURL(join(OUT, 'index.html')).href);
process.exit(failed.length ? 1 : 0);
