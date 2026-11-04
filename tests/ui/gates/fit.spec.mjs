// The chrome fits at any width: in every design and theme, with a song loaded, the window is narrowed from 2560 to
// 360 px and widened back, 20 px at a time, and the header, the player bar, the mini player and the tab bar are
// checked at each step (checks.mjs `chrome`): nothing outside its bar or the window, no control on another, no text
// cut off, every essential there. The header and the bar fit themselves (layout.js, data-fit), so no design needs a
// width band to patch them; a new one that would is caught here. The desktop app's mini player is checked at its
// own sizes. docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { chrome } from '../checks.mjs';
import { designs, ids, openScreen, themes } from './lib.mjs';

const widths = [];
for (let w = 2560; w >= 360; w -= 20) widths.push(w);
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
// the header and the bar as they stand, to name the cause when a width fails: each part's box and whether it gave way
const geometry = (page) => page.evaluate(() => 'fonts ' + document.fonts.status + ' | ' + ['.header', '#pbar'].map((sel) => {
  const row = document.querySelector(sel); if (!row) return '';
  // to a hundredth of a pixel: a row on the edge of fitting is decided by fractions; a row's children, and those of
  // its last (the tools, where the parts that give way sit)
  const box = (e) => { const r = e.getBoundingClientRect(); return r.left.toFixed(2) + '-' + r.right.toFixed(2); };
  const kids = (el) => [...el.children].map((c) => String(c.getAttribute('class') || c.tagName).split(' ')[0] + (c.hasAttribute('data-fit-out') ? ' (out)' : '') + ' ' + box(c));
  const last = row.lastElementChild;
  return sel + ' ' + box(row) + ' (scroll ' + row.scrollWidth + ' of ' + row.clientWidth + '): ' + kids(row).join(', ') + (last ? ' [in ' + String(last.className).split(' ')[0] + ': ' + kids(last).join(', ') + ']' : '');
}).join(' | '));

for (const design of designs) {
  for (const theme of themes) {
    test(`header and player bar / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');       // a song in the bar: its widest state
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForTimeout(500);
      const found = new Map(), seen = new Map();
      for (const w of [...widths, ...widths.slice().reverse()]) {    // both ways: a row fits again as it widens
        await page.setViewportSize({ width: w, height: 900 });
        await frames(page);
        const now = await chrome(page);
        if (now.length) {
          /* what the rows looked like then, and whether the next frame, then the settled page (fonts in), still
             overflow: a moment's overflow fails as one that stays does, and this names its cause */
          const at = await geometry(page);
          await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
          const next = await chrome(page);
          await page.evaluate(() => document.fonts.ready); await frames(page);
          const settled = await chrome(page);
          seen.set(w, `${at}\n    next frame: ${next.join('; ') || 'fits'}; settled: ${settled.join('; ') || 'fits'}`);
        }
        for (const p of now) found.set(p, [...(found.get(p) || []), w]);
      }
      const report = [...found].map(([p, ws]) => `${p} at ${[...new Set(ws)].sort((a, b) => a - b).join(', ')} px`);
      expect(report, 'the chrome at some width\n' + [...seen].map(([w, g]) => `at ${w} px: ${g}`).join('\n')).toEqual([]);
    });

    // the rows fit again by themselves when their text widens in place (a label that grew; no row changes size) and
    // when a font arrives with wider or narrower letters (a design's own, on first use): two frames on, they are as a
    // fresh fit of everything leaves them, and what gave way comes back when the narrower font does
    test(`header and player bar / ${design} / ${theme}: content that grows in place fits at once`, async ({ page }) => {
      // to name a failure's kind: how many times the page's resize observers ran, and what threw, since a change
      await page.addInitScript(() => {
        window.__ro = 0; window.__errors = [];
        const RO = window.ResizeObserver;
        window.ResizeObserver = class extends RO { constructor(cb) { super((entries, o) => { window.__ro++; try { cb(entries, o); } catch (e) { window.__errors.push(String(e && e.stack || e)); throw e; } }); } };
        window.addEventListener('error', (e) => window.__errors.push(String(e.message)));
      });
      const since = () => page.evaluate(() => { const r = `resize callbacks ${window.__ro}, errors ${JSON.stringify(window.__errors)}`; window.__ro = 0; return r; });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForTimeout(500);
      const out = () => page.evaluate(() => [...document.querySelectorAll('[data-fit-out]')].map((p) => p.getAttribute('class')).join(' / '));
      const fresh = async () => { await page.evaluate(() => App.layout.refit()); return out(); };
      // the rows set in a font of this name, as it arrives: one file under three names, the wide one with wider letters
      const font = (name) => page.evaluate(async (name) => {
        const face = new FontFace(name, 'url(/designs/glasshouse/assets/fonts/plex-mono-400-latin.woff2)', { sizeAdjust: name === 'gate-wide' ? '130%' : '100%' });
        document.fonts.add(face);
        let s = document.getElementById('gate-font');
        if (!s) { s = document.createElement('style'); s.id = 'gate-font'; document.head.appendChild(s); }
        s.textContent = `.header *, #pbar * { font-family: ${name} !important; }`;
        await face.load();
      }, name);
      for (const w of [1440, 1140, 960]) {
        await page.setViewportSize({ width: w, height: 900 });
        await frames(page);
        // a label that grew: wider text everywhere in the rows
        await since();
        await page.evaluate(() => { const s = document.createElement('style'); s.id = 'gate-wide'; s.textContent = '.header *, #pbar * { letter-spacing: .14em !important; }'; document.head.appendChild(s); });
        await frames(page);
        let now = await out(), at = await geometry(page), why = await since();
        let refit = await fresh(), atRefit = await geometry(page);
        expect(now, `${w} px, the text wider: fitted already (${at}; ${why}; after a fresh fit: ${atRefit})`).toEqual(refit);
        await page.evaluate(() => document.getElementById('gate-wide').remove());
        // fonts arriving: a narrow one, a wider one, a narrow one again
        await font(`gate-narrow-${w}`); await frames(page);
        const before = await out();
        expect(before, `${w} px, a font arrives: fitted already`).toEqual(await fresh());
        await since(); await font('gate-wide'); await frames(page);
        now = await out(); at = await geometry(page); why = await since();
        refit = await fresh(); atRefit = await geometry(page);
        expect(now, `${w} px, a wider font arrives: fitted already (${at}; ${why}; after a fresh fit: ${atRefit})`).toEqual(refit);
        await font(`gate-narrow-${w}-again`); await frames(page);
        now = await out();
        expect(now, `${w} px, the narrower font again: fitted already`).toEqual(await fresh());
        expect(now, `${w} px, the narrower font again: what gave way comes back`).toEqual(before);
        await page.evaluate(() => document.getElementById('gate-font').remove());
      }
    });

    test(`desktop mini player / ${design} / ${theme}`, async ({ page }) => {
      // the device window opens 340 px wide, the strip is a fixed 540 x 90 (app/src/desktop/mini.rs)
      for (const [view, width] of [['mini', 340], ['strip', 540]]) {
        await page.setViewportSize({ width, height: 700 });
        await page.goto(`/desktop/wrapper.html#view=${view}&design=${design}&theme=${theme}&opaque=1`);
        await page.waitForSelector('[data-dk-player]');
        await page.evaluate(() => document.fonts.ready);
        expect(await chrome(page), `the ${view} at ${width} px`).toEqual([]);
      }
    });
  }
}
