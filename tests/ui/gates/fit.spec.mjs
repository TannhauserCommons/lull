// The chrome works at any width: in every design and theme, with a song loaded, the window is narrowed from 2560 to
// 360 px and widened back, 20 px at a time, and the header, the player bar, the mini player and the tab bar are
// checked at each step (checks.mjs `chrome`): every control on the screen and reachable by a click, no text covered,
// on other text, off the screen or cut off, every feature reached (checks.mjs `features`, web/designs/features.json),
// and what the menus and sheets one click opens hold checked by opening them at a few widths; and Home's rows show every
// cover whole (`clippedCovers`). Function only, never form (ADR-0083, ADR-0101): where a part sits, whether it rises
// out of its bar or sits in another control's area, and the bar's shape are the design's. The header and the bar fit
// themselves (layout.js, data-fit), so no design needs a width band to patch them; a new one that would is caught here.
// The desktop app's mini player is checked at its own sizes. docs/context/quality.md#ui-gates.

import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { chrome, chromeNow, clippedCovers, FEATURES, features, gateFree, holders, unknownFeatures } from '../checks.mjs';
import { designs, ids, openScreen, settle, themes } from './lib.mjs';

const widths = [];
for (let w = 2560; w >= 360; w -= 20) widths.push(w);
const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
// a width as the page draws it: the frame whose resize observers saw it (its layout ran, and the fitter's observers with
// it), then one frame more. setViewportSize can return before the renderer takes the new size, so two frames counted
// from there may both be at the old one, and a check then forces a layout at a width no frame has drawn: rows not yet
// fitted, though no frame ever shows them so (watchObservers keeps window.__drawn)
// the chrome as the frame that first draws each width shows it: checked inside that frame, by a resize observer made after
// the page's own (layout.js's), so it runs once the fitter has fitted the rows for the width and before the frame is
// painted. A row a frame late shows here, however soon after it fits
const checkDrawn = (page) => page.evaluate(([src, list]) => {
  const check = (0, eval)('(' + src + ')');
  window.__drawnChrome = {};
  new ResizeObserver(() => { window.__drawnChrome[innerWidth] = check(null, { list }); }).observe(document.documentElement);
}, [chromeNow.toString(), FEATURES]);
const drawnAt = (page, w) => page.evaluate((w) => new Promise((r) => {
  const look = () => requestAnimationFrame(() => (window.__drawn === w ? requestAnimationFrame(() => r()) : look()));
  look();
}), w);
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

// to name a failure's kind: how many times the page's resize observers ran, what threw, and what changed in the header
// and the bar other than the fitter's own marks (a re-render, a state flip), since the last call of since(); and the
// width the page itself had then
const watchObservers = (page) => page.addInitScript(() => {
  window.__ro = 0; window.__errors = []; window.__changes = {};
  const drawn = new window.ResizeObserver(() => { window.__drawn = innerWidth; });
  if (document.documentElement) drawn.observe(document.documentElement);
  else new MutationObserver((r, o) => { if (document.documentElement) { o.disconnect(); drawn.observe(document.documentElement); } }).observe(document, { childList: true });
  const note = (records) => records.forEach((r) => {
    const t = r.target.nodeType === 1 ? r.target : r.target.parentElement; if (!t || !t.closest('.header, #pbar')) return;
    if (r.type === 'attributes' && /^data-fit-(out|gave)$/.test(r.attributeName)) return;
    if (r.type === 'attributes' && r.attributeName === 'style' && t.id === 'pbar') return;   // --fit-side
    const k = String(t.className && t.className.baseVal === undefined ? t.className : t.tagName).split(' ')[0] + ' ' + (r.type === 'attributes' ? r.attributeName : r.type);
    window.__changes[k] = (window.__changes[k] || 0) + 1;
  });
  document.addEventListener('DOMContentLoaded', () => new MutationObserver(note).observe(document.body, { subtree: true, attributes: true, childList: true, characterData: true }));
  const RO = window.ResizeObserver;
  window.ResizeObserver = class extends RO { constructor(cb) { super((entries, o) => { window.__ro++; try { cb(entries, o); } catch (e) { window.__errors.push(String(e && e.stack || e)); throw e; } }); } };
  window.addEventListener('error', (e) => window.__errors.push(String(e.message)));
});
const since = (page) => page.evaluate(() => {
  const r = `page width ${innerWidth}, resize callbacks ${window.__ro}, errors ${JSON.stringify(window.__errors)}, changes ${JSON.stringify(window.__changes)}`;
  window.__ro = 0; window.__errors = []; window.__changes = {}; return r;
});

for (const design of designs) {
  for (const theme of themes) {
    test(`header and player bar / ${design} / ${theme}`, async ({ page }) => {
      await watchObservers(page);
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');       // a song in the bar: its widest state
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForTimeout(500);
      await checkDrawn(page);
      const found = new Map(), seen = new Map();
      for (const w of [...widths, ...widths.slice().reverse()]) {    // both ways: a row fits again as it widens
        await since(page);
        await page.setViewportSize({ width: w, height: 900 });
        await drawnAt(page, w);
        // every feature reached too (checks.mjs features), less the findings features-known.json lists for this design
        const now = unknownFeatures((await page.evaluate((w) => window.__drawnChrome[w], w)) || await chrome(page), { design, width: w, screen: 'home' });
        if (now.length) {
          /* what the rows looked like then, and whether the next frame, then the settled page (fonts in), still
             overflow: a moment's overflow fails as one that stays does, and this names its cause */
          const at = `${await geometry(page)}; ${await since(page)}`;
          await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => r())));
          const next = await chrome(page);
          await page.evaluate(() => document.fonts.ready); await frames(page);
          const settled = await chrome(page);
          seen.set(w, `${at}\n    next frame: ${next.join('; ') || 'fits'}; settled: ${settled.join('; ') || 'fits'}`);
        }
        for (const p of now) found.set(p, [...(found.get(p) || []), w]);
        // Home's rows at this width: a cover's round window or ring cut by the shelf that scrolls it (checks.mjs)
        const cut = await clippedCovers(page);
        if (cut) found.set(cut, [...(found.get(cut) || []), w]);
      }
      const report = [...found].map(([p, ws]) => `${p} at ${[...new Set(ws)].sort((a, b) => a - b).join(', ')} px`);
      expect(report, 'the chrome at some width\n' + [...seen].map(([w, g]) => `at ${w} px: ${g}`).join('\n')).toEqual([]);
    });

    // what one click opens holds what features.json says (the features check trusts it at every width above): the
    // player's "…" menu and Following and playlists on a laptop and a tablet, the More sheet on a phone, each opened and
    // closed again; and on a phone Now Playing, the player's sheet there, with its own "…", and Settings' list, which
    // holds the design picker
    test(`what one click opens holds its features / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
      await page.waitForFunction(() => App.player.current());
      const found = [];
      for (const [w, route] of [[1440, '/home'], [800, '/home'], [390, '/home'], [390, '/now'], [390, '/settings']]) {
        await page.setViewportSize({ width: w, height: w < 640 ? 844 : 900 });
        await page.evaluate((r) => { location.hash = '#' + r; }, route);
        await settle(page);
        const screen = route.split('/')[1];
        for (const p of unknownFeatures([...await holders(page), ...await features(page)], { design, width: w, screen })) found.push(`${route} at ${w} px: ${p}`);
      }
      expect(found, 'what the holders hold').toEqual([]);
    });

    // nothing playing, private listening on: the song side holds only the Private pill, which shows whole or keeps
    // its eye, never clipped by the side's edge (tablet's song side clips what runs out of it)
    test(`player bar, nothing playing, private listening / ${design} / ${theme}`, async ({ page }) => {
      await watchObservers(page);
      await openScreen(page, '/home', { design, theme });
      await page.evaluate(() => App.store.set('privateListening', true));
      const found = new Map();
      for (let w = 1400; w >= 640; w -= 20) {
        await page.setViewportSize({ width: w, height: 900 });
        await drawnAt(page, w);
        for (const p of unknownFeatures(await chrome(page), { design, width: w, screen: 'home' })) found.set(p, [...(found.get(p) || []), w]);
        const clipped = await page.evaluate(() => { const n = document.querySelector('#pbar-now'); return n.scrollWidth > n.clientWidth + 1 ? `the song side runs ${n.scrollWidth - n.clientWidth} px past its edge` : ''; });
        if (clipped) found.set(clipped, [...(found.get(clipped) || []), w]);
      }
      expect([...found].map(([p, ws]) => `${p} at ${ws.join(', ')} px`), 'the idle bar with private listening on').toEqual([]);
    });

    // a laptop's window has room for the song's source and quality: at 1440 px its chips, and Like and Save, stay in the
    // bar (lull's song column once held its buttons inside its share of the room, and the chips gave way beside a
    // 366 px seek). What gave way is still in the "…" menu, so this is about room, not reach: a design whose bar is
    // narrower than the window on purpose names `song-side-room` in --gate-free on #pbar, with the reason (gateFree)
    test(`player bar, a song at 1440 px keeps its chips / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await page.waitForTimeout(500);
      await frames(page);
      const said = await page.evaluate((src) => (0, eval)('(' + src + ')')(document.querySelector('#pbar'), 'song-side-room'), gateFree.toString());
      test.skip(said === 'free', 'the design says why its song side has less room (--gate-free: song-side-room)');
      const out = await page.evaluate(() => [...document.querySelectorAll('#pbar-now :is(.pbar__badges > *, .pbar__acts)[data-fit-out]')].map((p) => String(p.className).split(' ')[0]));
      expect(out, `what gave way in the song side at 1440 px${said === 'no reason' ? ' (--gate-free names song-side-room without a reason: say why)' : ''}`).toEqual([]);
    });

    // the rows fit again by themselves when their text widens in place (a label that grew; no row changes size) and
    // when a font arrives with wider or narrower letters (a design's own, on first use): two frames on, they are as a
    // fresh fit of everything leaves them, and what gave way comes back when the narrower font does
    test(`header and player bar / ${design} / ${theme}: content that grows in place fits at once`, async ({ page }) => {
      await watchObservers(page);
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
        await drawnAt(page, w);
        // a label that grew: wider text everywhere in the rows
        await since(page);
        await page.evaluate(() => { const s = document.createElement('style'); s.id = 'gate-wide'; s.textContent = '.header *, #pbar * { letter-spacing: .14em !important; }'; document.head.appendChild(s); });
        await frames(page);
        let now = await out(), at = await geometry(page), why = await since(page);
        let refit = await fresh(), atRefit = await geometry(page);
        expect(now, `${w} px, the text wider: fitted already (${at}; ${why}; after a fresh fit: ${atRefit})`).toEqual(refit);
        await page.evaluate(() => document.getElementById('gate-wide').remove());
        // fonts arriving: a narrow one, a wider one, a narrow one again
        await font(`gate-narrow-${w}`); await frames(page);
        const before = await out();
        expect(before, `${w} px, a font arrives: fitted already`).toEqual(await fresh());
        await since(page); await font('gate-wide'); await frames(page);
        now = await out(); at = await geometry(page); why = await since(page);
        refit = await fresh(); atRefit = await geometry(page);
        expect(now, `${w} px, a wider font arrives: fitted already (${at}; ${why}; after a fresh fit: ${atRefit})`).toEqual(refit);
        await font(`gate-narrow-${w}-again`); await frames(page);
        now = await out();
        expect(now, `${w} px, the narrower font again: fitted already`).toEqual(await fresh());
        expect(now, `${w} px, the narrower font again: what gave way comes back`).toEqual(before);
        await page.evaluate(() => document.getElementById('gate-font').remove());
      }
    });

    // the phone's mini player while a song streams from Soulseek (the fixture's slow transfer): what a design draws
    // for that state (lull's "streaming" chip) fits beside the song's line or gives way, never over a button
    test(`phone mini player, a song streaming / ${design} / ${theme}`, async ({ page }) => {
      await watchObservers(page);
      await page.setViewportSize({ width: 620, height: 800 });
      await openScreen(page, '/home', { design, theme });
      const album = await page.evaluate(() => App.data.api('/api/feed').then((f) => f.newReleases[0]));
      await page.evaluate((id) => { location.hash = '#/album/' + id; }, album);
      const track = await page.waitForSelector('#view .trow[data-track-id]').then((r) => r.getAttribute('data-track-id'));
      await page.request.post('/__fixture/trickle', { data: { id: track, bytesPerS: 2000 } });
      await page.evaluate((id) => App.player.playTrack(id, [id]), track);
      await page.waitForSelector('#mini .seek--line.is-transferring');
      const found = new Map();
      for (let w = 620; w >= 360; w -= 20) {
        await page.setViewportSize({ width: w, height: 800 });
        await drawnAt(page, w);
        for (const p of unknownFeatures(await chrome(page), { design, width: w, screen: 'album' })) found.set(p, [...(found.get(p) || []), w]);
      }
      expect([...found].map(([p, ws]) => `${p} at ${ws.join(', ')} px`), 'the mini player while a song streams').toEqual([]);
    });

    // the side panel's title strip names the panel and carries its meta: on Lyrics the meta is the song's title, which
    // can be any length (lull's ran over the word "lyrics", Petal's wrapped over its window keys). Each tab, with a long
    // title playing: the meta's text is on the screen, clear of the title's text, not under anything, ellipsised where
    // it is cut, and the panel's own buttons can still be clicked. Where it sits and how many lines it takes are the
    // design's
    test(`side panel strip, a long song title / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { App.player.current().title = 'A Rusty Dream (from Cyberpunk: Edgerunners 2) - Extended Night Drive Version, Live at the Harbour'; location.hash = '#/home'; });
      await page.waitForTimeout(500);
      const found = [];
      for (const w of [2560, 1440, 1280]) {
        await page.setViewportSize({ width: w, height: 900 });
        await frames(page);
        for (const tab of ['queue', 'lyrics', 'activity']) {
          // the tab as a press picks it (app.js hears clicks on the document), whether or not the panel shows now
          if (!(await page.evaluate((tab) => { const b = document.querySelector(`[data-ctx-tab="${tab}"]`); if (b) b.click(); return !!b; }, tab))) continue;
          await frames(page);
          // judged once the panel has arrived (a panel that slides in is still off the screen mid-way)
          await page.evaluate(() => Promise.race([Promise.all(document.getAnimations().filter((a) => Number.isFinite(a.effect?.getComputedTiming().endTime)).map((a) => a.finished.catch(() => {}))), new Promise((r) => setTimeout(r, 3000))]));
          const p = await page.evaluate(() => {
            const shown = (e) => { if (!e) return null; const s = getComputedStyle(e), r = e.getBoundingClientRect(); return s.display !== 'none' && s.visibility !== 'hidden' && r.width > 0 && r.height > 0 && parseFloat(s.fontSize) > 0 ? r : null; };
            const meet = (a, b) => a.left < b.right - 0.5 && a.right > b.left + 0.5 && a.top < b.bottom - 0.5 && a.bottom > b.top + 0.5;
            // a text's lines as they show: cut to its own box (an ellipsis), and to their glyphs (lines set tighter than the
            // font's height have boxes that overlap where no letters do)
            const lines = (e) => {
              const rg = document.createRange(); rg.selectNodeContents(e); const b = e.getBoundingClientRect();
              return [...rg.getClientRects()].filter((r) => r.width > 0).map((r) => { const h = r.bottom - r.top; return { left: Math.max(r.left, b.left), right: Math.min(r.right, b.right), top: r.top + h * 0.25, bottom: r.bottom - h * 0.25 }; }).filter((r) => r.right - r.left >= 1);
            };
            // a click reaches it somewhere: its centre, else a grid of 5 x 5 points
            const reached = (k, r) => {
              for (const [i, j] of [[2, 2], ...[0, 1, 2, 3, 4].flatMap((a) => [0, 1, 2, 3, 4].map((b) => [a, b]))]) if (mine(k, document.elementFromPoint(r.left + (r.width * (i + 0.5)) / 5, r.top + (r.height * (j + 0.5)) / 5))) return true;
              return false;
            };
            const mine = (e, hit) => !!hit && (hit === e || e.contains(hit) || hit.contains(e));
            const ctx = shown(document.querySelector('#ctx')), m = document.querySelector('#ctx-meta'), mr = shown(m);
            if (!ctx || !mr || !m.textContent.trim()) return [];
            const out = [], t = document.querySelector('#ctx-title'), ml = lines(m);
            if (shown(t) && lines(t).some((a) => ml.some((b) => meet(a, b)))) out.push('the meta\'s text on the title\'s');
            if (ml.some((r) => r.left < -1 || r.right > innerWidth + 1 || r.top < -1 || r.bottom > innerHeight + 1)) out.push('the meta off screen');
            const c = ml[0] && document.elementFromPoint((ml[0].left + ml[0].right) / 2, (ml[0].top + ml[0].bottom) / 2);
            // what lies over it hides it only where it paints (an empty box stretched across the strip hides nothing)
            const paints = (hit) => { for (let a = hit; a && !a.contains(m); a = a.parentElement) { const s = getComputedStyle(a); if (/^(IMG|svg|CANVAS|VIDEO)$/.test(a.tagName) || s.backgroundImage !== 'none' || !/^(transparent|rgba\([^)]*,\s*0\))$/.test(s.backgroundColor)) return true; } return false; };
            if (c && getComputedStyle(m).pointerEvents !== 'none' && !mine(m, c) && paints(c)) out.push(`the meta under ${c.getAttribute('aria-label') || String(c.className).split(' ')[0]}`);
            for (const k of document.querySelectorAll('#ctx .ctx__head > .btn-icon')) {
              const kr = shown(k); if (!kr) continue;
              if (!reached(k, kr)) { const on = document.elementFromPoint((kr.left + kr.right) / 2, (kr.top + kr.bottom) / 2); out.push(`${k.getAttribute('aria-label')} can't be clicked, under ${on ? (on.getAttribute('aria-label') || String(on.className).split(' ')[0] || on.tagName) : 'nothing'}`); }
            }
            const s = getComputedStyle(m);
            if (m.scrollWidth > m.clientWidth + 1 && s.textOverflow !== 'ellipsis') out.push('the meta is cut without an ellipsis');
            return out;
          });
          for (const x of p) found.push(`${tab} at ${w} px: ${x}`);
        }
      }
      expect(found, 'the side panel strip with a long song title').toEqual([]);
    });

    // Settings' tab list on a short laptop window (a 1080p screen at 125%: 1536 x 864): beside a section that scrolls,
    // it fits in the view and its last tab can be reached and clicked, scrolling the list itself where it must, both at
    // the top of the page (the page's heading above it) and stuck mid-section (screens/settings-fit.js, --view-h and
    // --nav-lift). Before, its end hid under the player bar until the section had scrolled.
    test(`settings tab list on a short window / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 1536, height: 864 });
      await openScreen(page, '/settings/playback', { design, theme });
      for (const [where, at] of [['at the top of the page', 0], ['mid-section', 0.5]]) {
        await page.evaluate((at) => { const view = document.querySelector('#view'); view.scrollTop = (view.scrollHeight - view.clientHeight) * at; }, at);
        await frames(page);
        const seen = await page.evaluate(() => {
          const view = document.querySelector('#view'), nav = document.querySelector('.sys-nav');
          [nav, ...nav.querySelectorAll('*')].forEach((e) => { e.scrollTop = e.scrollHeight; });   // the list scrolled to its end, where it scrolls
          const v = view.getBoundingClientRect(), n = nav.getBoundingClientRect(), tabs = nav.querySelectorAll('.sys-nav__item');
          const tab = tabs[tabs.length - 1], last = tab.getBoundingClientRect();
          const hit = document.elementFromPoint((last.left + last.right) / 2, (last.top + last.bottom) / 2);
          return { scrolls: getComputedStyle(nav).position === 'sticky', fits: n.bottom <= v.bottom + 1,
            lastShows: last.top >= Math.max(v.top, n.top) - 1 && last.bottom <= Math.min(v.bottom, n.bottom) + 1, lastClicks: !!hit && tab.contains(hit) };
        });
        // a design that lets the list scroll with the page (not sticky) shows every tab on the way; a sticky one must fit
        if (seen.scrolls) expect(seen, `the tab list ${where} fits in the view and its last tab shows and takes a click`).toEqual({ scrolls: true, fits: true, lastShows: true, lastClicks: true });
      }
    });

    // Settings' choices on a phone at the largest text size: a choice whose buttons don't fit its row side by side wraps
    // (two to a row, else one; app.js fitSeg), so no choice runs past its row, and the indicator stays on the picked one,
    // which it shows; a design that draws it elsewhere on purpose (a dot beside it) names `seg-indicator` in --gate-free
    test(`settings choices on a phone at the largest text / ${design} / ${theme}`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await openScreen(page, '/settings/appearance', { design, theme });
      await page.evaluate(() => App.setPref('textSize', 125));
      await settle(page);
      await frames(page);
      const bad = await page.evaluate((src) => { const free = (0, eval)('(' + src + ')'); return [...document.querySelectorAll('.sys-seg')].flatMap((seg) => {
        const row = seg.closest('.sys-row').getBoundingClientRect(), key = seg.getAttribute('data-key'), out = [];
        const btns = [...seg.querySelectorAll('.seg__btn')], picked = btns.find((b) => b.getAttribute('aria-checked') === 'true');
        if (btns.some((b) => b.getBoundingClientRect().right > row.right + 1)) out.push(`${key}: a choice runs past its row`);
        const ind = seg.querySelector('.seg__ind');
        const said = ind ? free(ind, 'seg-indicator') : '';
        if (picked && ind && getComputedStyle(ind).display !== 'none' && said !== 'free') {
          const i = ind.getBoundingClientRect(), b = picked.getBoundingClientRect();
          // across it the picked button's width; down, from its top to just below it (a design may draw it as a line underneath)
          if (Math.abs(i.left - b.left) > 2 || Math.abs(i.right - b.right) > 2 || i.top < b.top - 2 || i.top > b.bottom + 8) out.push(`${key}: the indicator is off the picked choice${said === 'no reason' ? ' (--gate-free names seg-indicator without a reason: say why)' : ''}`);
        }
        return out;
      }); }, gateFree.toString());
      expect(bad).toEqual([]);
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

// the desktop app's window on Windows draws its own minimise, maximise and close buttons over the page's top right corner
// (desktop.js, 138 x 32 px): the header keeps clear of them at every width, whatever a design sets for the header's
// margin and padding (desktop.css), with the chrome checks as above. lull's header once ran under them
for (const design of designs) {
  test(`header beside the desktop window's buttons / ${design}`, async ({ page }) => {
    await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
    await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
    await page.setViewportSize({ width: 1440, height: 900 });
    await openScreen(page, `/album/${ids.album}`, { design });
    await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
    await page.waitForFunction(() => App.player.current());
    await page.evaluate(() => { location.hash = '#/home'; });
    await page.waitForSelector('.dk-caption');
    const found = new Map();
    for (let w = 2560; w >= 480; w -= 40) {     // the window is never narrower than 480 px (app/src/desktop/main_window.rs)
      await page.setViewportSize({ width: w, height: 900 });
      await frames(page);
      const under = await page.evaluate(() => {
        const c = document.querySelector('.dk-caption').getBoundingClientRect();
        return [...document.querySelectorAll('.header :is(a, button, input, [tabindex], .tray)')].filter((e) => {
          const r = e.getBoundingClientRect(), s = getComputedStyle(e);
          return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && r.right > c.left + 0.5 && r.left < c.right && r.top < c.bottom && r.bottom > c.top;
        }).map((e) => `${e.getAttribute('aria-label') || String(e.className).split(' ')[0]} under the window's buttons`);
      });
      for (const p of [...under, ...unknownFeatures(await chrome(page), { design, width: w, screen: 'home' })]) found.set(p, [...(found.get(p) || []), w]);
    }
    expect([...found].map(([p, ws]) => `${p} at ${ws.join(', ')} px`), "the header in the desktop app's window").toEqual([]);
  });
}

// the search and the seek are the main tools of the header and the player bar: they keep a comfortable width before the
// status tray, the bar's secondary keys and the volume slider give way (layout.js: the search's own --search-min floor
// and the fit order, --fit-seek-min), whatever a design lays out, in a browser and in the desktop app's window (whose
// buttons take room from the header). The tray shows full while the search has 220 px to type in (the tray compacts
// first), the seek track has 360 px (300 on a tablet) while any secondary key still shows, and the bar shows its time
// once, "0:00 / 3:19", the seek's aria-valuetext still saying both
for (const design of designs) {
  for (const desktop of [false, true]) {
    test(`the search and the seek keep their room / ${design} / ${desktop ? 'desktop app' : 'browser'}`, async ({ page }) => {
      if (desktop) {
        await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
        await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
      }
      await page.setViewportSize({ width: 1440, height: 900 });
      await openScreen(page, `/album/${ids.album}`, { design });
      await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
      await page.waitForFunction(() => App.player.current());
      await page.evaluate(() => { location.hash = '#/home'; });
      await settle(page);
      const found = [];
      for (const w of [1024, 1280, 1460, 1920]) {
        await page.setViewportSize({ width: w, height: 900 });
        await page.fill('#header-search', 'odetari blackhole');
        await frames(page); await frames(page);
        const m = await page.evaluate(() => {
          const shown = (e) => e.getClientRects().length > 0;
          const input = document.querySelector('#header-search'), cs = getComputedStyle(input);
          const inner = input.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
          const seek = document.querySelector('.pbar__seek .seek').getBoundingClientRect().width;
          const keys = [...document.querySelectorAll('.pbar__tools [data-fit]:not(.seek)')].filter(shown).length;
          return {
            inner, full: shown(document.querySelector('.tray--full')), seek, keys,
            vol: shown(document.querySelector('.vol > .seek')) && getComputedStyle(document.querySelector('.vol > .seek')).position !== 'absolute',
            times: [...document.querySelectorAll('#pbar .seek-row__time')].filter(shown).length,
            text: document.querySelector('.pbar__seek .seek').getAttribute('aria-valuetext') || '',
            readout: document.querySelector('#pbar .seek-row__time').textContent.replace(/\s+/g, ' ').trim(),
          };
        });
        if (m.full && m.inner < 220) found.push(`at ${w} px the search has ${Math.round(m.inner)} px to type in beside the full tray, under 220`);
        const least = w <= 1099 ? 300 : 360;
        if ((m.keys > 0 || m.vol) && m.seek < least) found.push(`at ${w} px the seek track is ${Math.round(m.seek)} px while ${m.keys} secondary keys${m.vol ? ' and the volume slider' : ''} show, under ${least}`);
        if (m.times !== 1) found.push(`at ${w} px the bar shows ${m.times} time readouts, not one`);
        if (!/ of /.test(m.text) || !/\d:\d\d \/ \d:\d\d/.test(m.readout.replace(' of ', ' ').replace(/(\d:\d\d) (\d:\d\d)/, '$1 / $2'))) found.push(`at ${w} px the readout "${m.readout}" or the seek's value text "${m.text}" lacks the length`);
      }
      expect(found, 'the search and the seek').toEqual([]);
    });
  }
}

// a theme that lays out round the window's buttons itself (--layout-window-controls: own, docs/themes.md section 5):
// lull reserves no room, so the header runs to the window's edge under them; their box is in --window-controls-w and -h,
// and they stay on top and clickable. At 1440 (the header's room) and 720 (the strip of their own above the shell)
test('a theme owns the room under the desktop window\'s buttons', async ({ page }) => {
  await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
  await page.addInitScript(() => { window.__LULL_DESKTOP__ = { os: 'win', titlebar: 'win' }; });
  await page.setViewportSize({ width: 1440, height: 900 });
  await openScreen(page, '/home', { design: 'lull' });
  await page.waitForSelector('.dk-caption');
  const read = () => page.evaluate(() => {
    const app = document.querySelector('#app'), header = document.querySelector('.header'), hs = getComputedStyle(header);
    const root = getComputedStyle(document.documentElement), c = document.querySelector('.dk-caption__btn--close').getBoundingClientRect();
    const hit = document.elementFromPoint(c.left + c.width / 2, c.top + c.height / 2);
    return {
      own: app.getAttribute('data-window-controls'), pad: hs.paddingRight, margin: hs.marginRight, top: getComputedStyle(app).paddingTop,
      w: root.getPropertyValue('--window-controls-w').trim(), h: root.getPropertyValue('--window-controls-h').trim(),
      close: !!(hit && hit.closest('.dk-caption__btn--close')),
    };
  });
  const before = await read();
  expect(before.own).toBeNull();
  expect(before).toMatchObject({ w: '138px', h: '32px', close: true });
  expect(parseFloat(before.pad) + parseFloat(before.margin)).toBeGreaterThanOrEqual(138);
  await page.addStyleTag({ content: '.app { --layout-window-controls: own; } .header { padding-right: 12px; }' });
  await page.setViewportSize({ width: 1439, height: 900 });   // lull reads its switches again on a resize
  await frames(page);
  await expect.poll(async () => (await read()).own).toBe('own');
  const after = await read();
  expect(after).toMatchObject({ pad: '12px', margin: '0px', w: '138px', h: '32px', close: true });
  await page.setViewportSize({ width: 720, height: 900 });
  await frames(page);
  const strip = await read();
  expect(strip).toMatchObject({ own: 'own', close: true });
  expect(strip.top, 'no strip of their own above the shell').not.toBe('32px');
});
