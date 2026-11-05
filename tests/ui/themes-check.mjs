// A quick look at screens in every theme, for a UI change: `mise run themes:check -- <route> [<route>...]`.
// Each route is drawn with a song in the player, on desktop (1440×900) and phone (390×844), day and night, in the five
// built-in designs and in every theme package in the folder LULL_THEMES_DIR names (a folder of `<id>/theme.json`
// folders; none: the built-ins only). Each picture runs the checks the gates run (checks.mjs: a control outside its
// bar or the window, two on each other, text cut off, an essential missing; anything sticking out sideways; a clamp
// that doesn't clip; a squeezed title; console errors), and one contact sheet shows them all:
// tests/ui/themes-check/index.html. It exits 1 when a check finds something. A route is a key screen's name (home,
// search, playlist, album, now, settings) or a path (/settings/appearance). On the fixture hub, with a copy of the
// theme folder, so nothing is written to it. A few minutes at most; the gates are the full matrix.

import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { chromium } from '@playwright/test';
import { chrome, overflow, squeezed, unclippedClamps } from './checks.mjs';
import { designs as BUILT_IN, ids, openScreen, ownHub, screens } from './gates/lib.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const OUT = join(here, 'themes-check');
const VIEWPORTS = { desktop: { viewport: { width: 1440, height: 900 } }, phone: { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true } };
const PARALLEL = 4;

const routes = process.argv.slice(2).map((r) => screens[r] || (r.startsWith('/') ? r : '/' + r));
if (!routes.length) {
  console.error(`usage: mise run themes:check -- <route> [<route>...]   (a path, or one of ${Object.keys(screens).join(', ')})`);
  process.exit(2);
}

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
      mine.push({ id: m.id, modes: Array.isArray(m.modes) && m.modes.length ? m.modes : ['day', 'night'] });
    } catch (e) { console.warn(`skipped ${entry.name}: ${e.message}`); }
  }
} else console.warn('LULL_THEMES_DIR is not set: the five built-in designs only');

const looks = [...BUILT_IN.map((id) => ({ id, modes: ['day', 'night'] })), ...mine];
const jobs = routes.flatMap((route) => looks.flatMap((d) => Object.keys(VIEWPORTS).flatMap((size) => d.modes.map((mode) => ({ route, design: d.id, size, mode })))));
const slug = (j) => `${j.route.replace(/[^\w]+/g, '-').replace(/^-|-$/g, '') || 'home'}--${j.design}--${j.size}--${j.mode}`;

rmSync(OUT, { recursive: true, force: true });
mkdirSync(OUT, { recursive: true });
const started = Date.now();
const hub = await ownHub({ FIXTURE_THEMES: copy });
const browser = await chromium.launch();

/* one picture: the route in that design, size and mode, with a song in the player, and what the checks find */
async function shoot(job) {
  const context = await browser.newContext({ baseURL: hub.url, reducedMotion: 'reduce', ...VIEWPORTS[job.size] });
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push('page error: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') errors.push('console: ' + m.text()); });
  try {
    await openScreen(page, `/album/${ids.album}`, { design: job.design, theme: job.mode });
    await page.evaluate(() => { const row = document.querySelector('#view .trow[data-track-id]'); return row && App.player.playTrack(row.getAttribute('data-track-id'), [row.getAttribute('data-track-id')]); });
    await page.waitForFunction(() => App.player.current(), null, { timeout: 5000 }).catch(() => errors.push('no song started'));
    await page.evaluate((r) => { location.hash = '#' + r; }, job.route);
    await page.waitForTimeout(400);
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(300);
    const found = [...(await chrome(page)), await overflow(page), await unclippedClamps(page), await squeezed(page)].filter(Boolean).map(String);
    await page.screenshot({ path: join(OUT, slug(job) + '.jpg'), type: 'jpeg', quality: 72 });
    return { ...job, problems: [...found, ...errors] };
  } catch (e) {
    return { ...job, problems: [...errors, 'could not draw it: ' + e.message.split('\n')[0]] };
  } finally { await context.close(); }
}

const results = [];
try {
  let next = 0;
  await Promise.all(Array.from({ length: PARALLEL }, async () => {
    while (next < jobs.length) {
      const job = jobs[next++];
      const r = await shoot(job);
      results.push(r);
      process.stdout.write(r.problems.length ? 'x' : '.');
    }
  }));
} finally { await browser.close(); hub.close(); rmSync(copy, { recursive: true, force: true }); }
process.stdout.write('\n');

/* the contact sheet: a row per design, a column per size and mode, under each route; what a check found under its picture */
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => `&#${c.charCodeAt(0)};`);
const cols = Object.keys(VIEWPORTS).flatMap((size) => ['day', 'night'].map((mode) => ({ size, mode })));
const failed = results.filter((r) => r.problems.length);
const cell = (r) => !r ? '<td class="none">no such mode</td>'
  : `<td class="${r.problems.length ? 'bad' : ''}"><a href="${slug(r)}.jpg"><img src="${slug(r)}.jpg" alt="${esc(`${r.design}, ${r.size}, ${r.mode}`)}" loading="lazy"></a>${r.problems.length ? `<ul>${r.problems.map((p) => `<li>${esc(p)}</li>`).join('')}</ul>` : ''}</td>`;
writeFileSync(join(OUT, 'index.html'), `<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Themes check</title>
<style>
  :root { color-scheme: light dark; --bg: #fff; --fg: #1d1d1f; --line: #d0d0d6; --bad: #c62828; }
  @media (prefers-color-scheme: dark) { :root { --bg: #17171a; --fg: #ececf0; --line: #3a3a42; --bad: #ff6b6b; } }
  body { margin: 16px; background: var(--bg); color: var(--fg); font: 14px/1.4 system-ui, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 4px; } h2 { font-size: 16px; margin: 24px 0 8px; }
  table { border-collapse: collapse; width: 100%; table-layout: fixed; }
  th, td { border: 1px solid var(--line); padding: 6px; vertical-align: top; text-align: left; }
  th[scope=row] { width: 110px; } img { display: block; width: 100%; height: auto; }
  td.bad { outline: 2px solid var(--bad); outline-offset: -2px; } td.bad ul { color: var(--bad); margin: 6px 0 0; padding-left: 18px; font-size: 12px; }
  td.none { color: GrayText; font-size: 12px; }
</style>
<h1>Themes check</h1>
<p>${results.length} pictures, ${failed.length ? `<strong style="color:var(--bad)">${failed.length} with something to look at</strong>` : 'nothing found by the checks'}; ${Math.round((Date.now() - started) / 1000)} s. Built-in designs, then the themes in LULL_THEMES_DIR, with a song in the player.</p>
${routes.map((route) => `<h2>${esc(route)}</h2><table><thead><tr><th></th>${cols.map((c) => `<th>${c.size}, ${c.mode}</th>`).join('')}</tr></thead><tbody>
${looks.map((d) => `<tr><th scope="row">${esc(d.id)}${mine.some((m) => m.id === d.id) ? '' : '<br><small>built in</small>'}</th>${cols.map((c) => cell(results.find((r) => r.route === route && r.design === d.id && r.size === c.size && r.mode === c.mode))).join('')}</tr>`).join('\n')}
</tbody></table>`).join('\n')}
`);

console.log(`${results.length} pictures in ${Math.round((Date.now() - started) / 1000)} s; ${failed.length ? failed.length + ' with something to look at' : 'nothing found'}`);
for (const r of failed) console.log(`  ${r.route} ${r.design} ${r.size} ${r.mode}: ${r.problems.join('; ')}`);
console.log('contact sheet: ' + pathToFileURL(join(OUT, 'index.html')).href);
process.exit(failed.length ? 1 : 0);
