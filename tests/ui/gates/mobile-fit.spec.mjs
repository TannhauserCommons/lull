// lull on a phone, as an installed web app or in the phone's browser (projects `mobile`, Chromium as Android, and
// `mobile-webkit`, WebKit as an iPhone): every screen keeps inside the window, nothing a listener reads or taps sits
// under the notch, the home indicator or the side of a phone held sideways, a phone held sideways leaves the page room,
// no field makes iOS zoom the page, and no text is too small to read.
// A test marked fixme('Mnn: ...') is a known problem, by its id in the mobile audit, asserted as it should be: the fix
// removes the mark. docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, ids } from './lib.mjs';
import { go, insets, isWebkit, open, withSong } from './mobile.mjs';

/* what reaches past the window's right edge, outside anything that scrolls or clips sideways */
const pastTheEdge = () => {
  const W = innerWidth, out = [];
  const clipped = (el) => { for (let e = el.parentElement; e && e !== document.body; e = e.parentElement) if (getComputedStyle(e).overflowX !== 'visible') return true; return false; };
  for (const el of document.querySelectorAll('body *')) {
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.right <= W + 1 || r.left >= W || clipped(el)) continue;
    const cs = getComputedStyle(el); if (cs.visibility === 'hidden' || cs.display === 'none' || el.closest('.sky-specks, .vh')) continue;
    out.push(`${el.tagName.toLowerCase()}.${[...el.classList].join('.')} reaches ${Math.round(r.right)}`);
  }
  return { page: document.documentElement.scrollWidth - W, past: out.slice(0, 5) };
};

/* the controls and titles drawn into the safe-area insets the page was given (viewport-fit=cover) */
const underInsets = () => {
  const p = document.createElement('div');
  p.style.cssText = 'position:fixed;top:env(safe-area-inset-top);left:env(safe-area-inset-left);right:env(safe-area-inset-right);bottom:env(safe-area-inset-bottom);visibility:hidden;pointer-events:none';
  document.body.append(p); const s = p.getBoundingClientRect(); p.remove();
  const W = innerWidth, H = innerHeight, out = [];
  for (const el of document.querySelectorAll('a[href], button, [role="slider"], [role="tab"], input, h1, .mini__title, .tab span')) {
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    if (r.width < 1 || r.height < 1 || cs.visibility === 'hidden' || el.closest('[hidden], .vh, .skip-link, .sky-specks')) continue;
    if (r.bottom <= 0 || r.top >= H || r.right <= 0 || r.left >= W) continue;   // scrolled out of sight
    let o = 1; for (let e = el; e; e = e.parentElement) o *= +getComputedStyle(e).opacity; if (o < 0.05) continue;
    const at = [r.top < s.top - 1 && 'top', r.bottom > s.bottom + 1 && 'bottom', r.left < s.left - 1 && 'left', r.right > s.right + 1 && 'right'].filter(Boolean);
    if (!at.length) continue;
    // in a list that can still scroll it clear, it isn't stuck there (the list's end is checked scrolled to the end)
    let sc = el.parentElement;
    while (sc && !(/(auto|scroll)/.test(getComputedStyle(sc).overflowY) && sc.scrollHeight > sc.clientHeight + 1)) sc = sc.parentElement;
    if (sc && ((at.includes('bottom') && sc.scrollTop + sc.clientHeight < sc.scrollHeight - 1) || (at.includes('top') && sc.scrollTop > 0))) continue;
    // drawn there, not scrolled under a bar that covers it: a point of it inside the inset reaches it
    const mid = (a, b) => (a + b) / 2;
    const x = at.includes('left') ? mid(Math.max(r.left, 0), Math.min(r.right, s.left)) : at.includes('right') ? mid(Math.max(r.left, s.right), Math.min(r.right, W)) : mid(r.left, r.right);
    const y = at.includes('top') ? mid(Math.max(r.top, 0), Math.min(r.bottom, s.top)) : at.includes('bottom') ? mid(Math.max(r.top, s.bottom), Math.min(r.bottom, H)) : mid(r.top, r.bottom);
    const hit = document.elementFromPoint(x, y);
    if (!hit || !(el === hit || el.contains(hit) || hit.contains(el))) continue;
    out.push(`${el.tagName.toLowerCase()}.${[...el.classList].slice(0, 2).join('.')} "${(el.getAttribute('aria-label') || el.textContent || '').trim().slice(0, 30)}" under the ${at.join('+')} inset`);
  }
  return out;
};

/* every list on the screen scrolled to its end, as a listener reaches the last row */
const toTheEnd = async () => {
  for (const el of document.querySelectorAll('*')) if (/(auto|scroll)/.test(getComputedStyle(el).overflowY) && el.scrollHeight > el.clientHeight + 1) el.scrollTop = el.scrollHeight;
  await new Promise((ok) => setTimeout(ok, 300));
};

const routes = ['/home', '/search?q=velvet', '/library/albums', `/album/${ids.album}`, `/artist/${ids.artist}`,
  `/playlist/${ids.playlist}`, '/now?tab=art', '/now?tab=queue', '/settings', '/settings/playback', '/playlists'];

test.describe('the smallest phone (360x780)', () => {
  test.use({ viewport: { width: 360, height: 780 } });
  for (const design of designs) {
    test(`no screen is wider than the phone / ${design}`, async ({ page }) => {
      await withSong(page, design);
      for (const route of routes) {
        await go(page, route);
        const o = await page.evaluate(pastTheEdge);
        expect(o.past, `${route}: past the right edge`).toEqual([]);
        expect(o.page, `${route}: the page scrolls sideways`).toBeLessThanOrEqual(0);
      }
    });
  }
});

// iPhone 14/15 in portrait: 47 px of notch above, 34 px of home indicator below
test.describe('the notch and the home indicator', () => {
  for (const design of designs) {
    test(`nothing to read or tap under them / ${design}`, async ({ page, browser }) => {
      test.skip(isWebkit(browser), 'WebKit takes no safe-area insets from a test');
      test.fixme(design === 'glasshouse', 'M11: Glasshouse\'s phone header and tab bar set their own padding without the safe-area insets');
      await insets(page, { top: 47, bottom: 34 });
      await withSong(page, design);
      for (const route of ['/home', '/now?tab=art', '/now?tab=queue', '/settings']) {
        await go(page, route);
        expect(await page.evaluate(underInsets), route).toEqual([]);
        await page.evaluate(toTheEnd);
        expect(await page.evaluate(underInsets), `${route}, scrolled to the end`).toEqual([]);
      }
      await go(page, '/home');
      await page.locator('.tabbar [data-act="more-sheet"]').tap();
      await page.waitForTimeout(600);
      expect(await page.evaluate(underInsets), 'the More sheet').toEqual([]);
    });
  }
});

// held sideways (844x390): the notch is on the left or right, 47 px, and the home indicator 21 px below
test.describe('a phone held sideways', () => {
  test.use({ viewport: { width: 844, height: 390 } });
  for (const design of designs) {
    test(`nothing to read or tap beside the notch / ${design}`, async ({ page, browser }) => {
      test.skip(isWebkit(browser), 'WebKit takes no safe-area insets from a test');
      test.fixme(true, 'M06: nothing pads for safe-area-inset-left/right, so the header, rail and player bar run under the notch');
      await insets(page, { bottom: 21, left: 47, right: 47 });
      await withSong(page, design);
      for (const route of ['/home', `/album/${ids.album}`, '/now']) {
        await go(page, route);
        expect(await page.evaluate(underInsets), route).toEqual([]);
      }
    });
    test(`the page keeps room to read, and notes stay on the screen / ${design}`, async ({ page }) => {
      test.fixme(design !== 'skydeck', 'M13: sideways, a phone gets the tablet layout: the header and player bar leave the page 13-45% of the height');
      await withSong(page, design, `/album/${ids.album}`);
      const view = await page.evaluate(() => document.querySelector('#view').getBoundingClientRect().height / innerHeight);
      expect(view, 'the share of the window the page gets beside the header and player').toBeGreaterThanOrEqual(0.5);
      await page.evaluate(() => { App.toast('Added to the queue'); App.toast('Saved to your library'); });
      await page.waitForTimeout(600);
      const out = await page.evaluate(() => [...document.querySelectorAll('#toasts > *')].map((t) => t.getBoundingClientRect())
        .filter((r) => r.top < -1 || r.bottom > innerHeight + 1).map((r) => `${Math.round(r.top)}..${Math.round(r.bottom)}`));
      expect(out, 'notes cut off by the window').toEqual([]);
    });
    test(`no screen is wider than the phone / ${design}`, async ({ page }) => {
      await withSong(page, design);
      for (const route of ['/home', `/album/${ids.album}`, '/now', '/settings/playback']) {
        await go(page, route);
        const o = await page.evaluate(pastTheEdge);
        expect(o.past, `${route}: past the right edge`).toEqual([]);
        expect(o.page, `${route}: the page scrolls sideways`).toBeLessThanOrEqual(0);
      }
    });
  }
});

/* every field a listener types in, at the size iOS reads it at: under 16 px, iOS zooms the page in on focus and leaves
   it zoomed after */
const smallFields = () => [...document.querySelectorAll('input, textarea, select')]
  .filter((el) => !/^(hidden|checkbox|radio|range|file|color)$/.test(el.type) && el.getClientRects().length && !el.closest('[hidden]'))
  .map((el) => ({ field: el.id || el.className, px: parseFloat(getComputedStyle(el).fontSize) }))
  .filter((f) => f.px < 16);

for (const design of designs) {
  test(`no field makes iOS zoom the page / ${design}`, async ({ page }) => {
    test.fixme(design === 'lull', 'M05: lull\'s fields are 15 px, so iOS zooms in on focus and stays zoomed');
    await open(page, '/search', { design });
    for (const route of ['/search', '/playlists', '/settings/appearance', '/settings/sources', '/settings/library']) {
      await go(page, route);
      expect(await page.evaluate(smallFields), route).toEqual([]);
    }
  });
}

test('the page can still be zoomed by the listener', async ({ page }) => {
  await open(page, '/home');
  const vp = await page.locator('meta[name="viewport"]').getAttribute('content');
  expect(vp).toContain('viewport-fit=cover');
  expect(vp, 'zoom is the listener\'s: no user-scalable=no or maximum-scale').not.toMatch(/user-scalable\s*=\s*(no|0)|maximum-scale/);
});

/* text a phone can read: nothing visible on the main screens under 11 px */
const tinyText = () => {
  const out = new Set(), H = innerHeight;
  const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n; (n = walk.nextNode());) {
    const el = n.parentElement;
    if (!n.textContent.trim() || !el || el.closest('.vh, [hidden], [aria-hidden="true"], script, style')) continue;
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    if (r.width < 1 || r.bottom < 0 || r.top > H || cs.visibility === 'hidden' || parseFloat(cs.fontSize) >= 11 || parseFloat(cs.fontSize) === 0) continue;   // 0 px: words kept for a screen reader
    let o = 1; for (let e = el; e; e = e.parentElement) o *= +getComputedStyle(e).opacity;
    if (o > 0.05) out.add(`${el.className} "${n.textContent.trim().slice(0, 20)}" ${cs.fontSize}`);
  }
  return [...out];
};

for (const design of designs) {
  test(`no text under 11 px / ${design}`, async ({ page }) => {
    await withSong(page, design, '/home');
    for (const route of ['/home', `/album/${ids.album}`, '/now?tab=art', '/now?tab=queue', '/settings/playback']) {
      await go(page, route);
      expect(await page.evaluate(tinyText), route).toEqual([]);
    }
  });
}
