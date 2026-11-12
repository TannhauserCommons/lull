// lull on a phone, by touch (projects `mobile` and `mobile-webkit`): the controls of the phone's own chrome are big
// enough for a thumb (Apple's 44x44 pt), Now Playing's play key is whole and on top, the Search tab is ready to type in,
// a row's menu opens without a mouse (its "…" visible, and a held touch), nothing waits for a hover a phone never
// sends, axe finds nothing new at the phone's width, and a note never lands on Now Playing's transport. A test marked fixme('Mnn: ...') is a known problem by its id in the mobile audit; its fix removes the mark.
// docs/context/quality.md#ui-gates.

import { readFileSync } from 'node:fs';
import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { designs, ids } from './lib.mjs';
import { go, isWebkit, open, withSong } from './mobile.mjs';

/* the controls a thumb reaches for: the header's, the tab bar's, Now Playing's transport and close (the mini player's
   are below) */
const CHROME = ['.header .btn-icon', '.header [data-act="status"]', '.tabbar .tab',
  '.sheet-bar [data-act="back"]', '#view [data-act="prev"]', '#view [data-act="play"]', '#view [data-act="next"]'];
const MINI = ['#mini button', '#mini .bezel-thumb'];
/* a control a thumb can hit: 44x44 or more, or a smaller look whose touch area reaches 21 px from its centre each way
   it is short, a tap there landing on the control itself, its words or a pseudo-element it draws, never a neighbour
   (elementFromPoint reads whole pixels, so 21 px from a centre that falls between pixels can land 22 px out) */
const tooSmall = (sels) => {
  const named = (el) => `${el.tagName.toLowerCase()}.${[...el.classList].slice(0, 2).join('.')}[${el.getAttribute('data-act') || el.getAttribute('data-nav') || ''}]`;
  const out = [];
  for (const el of document.querySelectorAll(sels.join(','))) {
    const r = el.getBoundingClientRect(), cs = getComputedStyle(el);
    if (r.width < 1 || cs.visibility === 'hidden' || el.closest('[hidden]')) continue;
    const x = r.left + r.width / 2, y = r.top + r.height / 2, R = 21;
    const probes = [...(r.width < 43.5 ? [[x - R, y], [x + R, y]] : []), ...(r.height < 43.5 ? [[x, y - R], [x, y + R]] : [])];
    const missed = probes.filter(([px, py]) => { const hit = document.elementFromPoint(px, py); return !hit || !(hit === el || el.contains(hit)); });
    if (missed.length) out.push(`${named(el)} ${Math.round(r.width)}x${Math.round(r.height)}, a tap ${missed.map(([px, py]) => `${Math.round(px - x)},${Math.round(py - y)}`).join(' / ')} px from its centre misses it`);
  }
  return [...new Set(out)];
};

for (const design of designs) {
  test(`the phone's controls are big enough for a thumb / ${design}`, async ({ page }) => {
    await withSong(page, design, '/home');
    const small = await page.evaluate(tooSmall, CHROME);
    await go(page, '/now?tab=art');
    small.push(...await page.evaluate(tooSmall, CHROME));
    expect(small, 'under 44x44').toEqual([]);
  });

  test(`the mini player's controls are big enough for a thumb / ${design}`, async ({ page }) => {
    test.fixme(true, 'M08: the mini player\'s keys are 36-40 px (fix-phone-bar gives them --hit)');
    await withSong(page, design, '/home');
    expect(await page.evaluate(tooSmall, MINI), 'under 44x44').toEqual([]);
  });

  // a listener's report (2026-11): the play key on Now Playing was squeezed to a sliver on an iPhone
  test(`Now Playing's play key is whole, round and on top / ${design}`, async ({ page }) => {
    test.fixme(design === 'lull', 'M03: lull\'s .lull-dev__screen has no height on a phone, so it clips the play key away');
    await withSong(page, design, '/now?tab=art');
    const key = await page.evaluate(() => {
      const b = [...document.querySelectorAll('#view [data-act="play"]')].find((e) => e.getClientRects().length);
      if (!b) return null;
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      // what clips it: an ancestor that hides overflow and ends inside the key
      let cut = null;
      for (let e = b.parentElement; e; e = e.parentElement) {
        if (getComputedStyle(e).overflow === 'visible') continue;
        const c = e.getBoundingClientRect();
        if (c.top > r.top + 2 || c.bottom < r.bottom - 2 || c.left > r.left + 2 || c.right < r.right - 2) { cut = e.className; break; }
      }
      return { w: r.width, h: r.height, onTop: !!hit && b.contains(hit), cut };
    });
    expect(key, 'a play key on Now Playing').not.toBeNull();
    expect(key.cut, 'nothing clips it').toBeNull();
    expect(key.onTop, 'a tap at its centre reaches it').toBe(true);
    expect(Math.min(key.w, key.h), 'at least 44 px').toBeGreaterThanOrEqual(44);
    expect(key.w / key.h, 'as wide as it is tall').toBeGreaterThan(0.85);
    expect(key.w / key.h).toBeLessThan(1.18);
  });

  // a listener's report (2026-11): the Search tab opened the page without putting the cursor in the field
  test(`the Search tab puts the cursor in the field / ${design}`, async ({ page }) => {
    test.fixme(true, 'M04: the Search tab only routes to #/search; nothing focuses #disc-q in the tap');
    await open(page, '/home', { design });
    await page.locator('.tabbar [data-nav="search"]').tap();
    await expect(page.locator('#disc-q')).toBeFocused();
  });

  test(`a song's "…" shows without a hover and opens its menu / ${design}`, async ({ page }) => {
    await open(page, `/playlist/${ids.playlist}`, { design });
    const more = page.locator('#view .trow[data-track-id] [aria-haspopup="menu"]').first();
    await expect(more).toBeVisible();
    expect(await more.evaluate((el) => { let o = 1; for (let e = el; e; e = e.parentElement) o *= +getComputedStyle(e).opacity; return o; }), 'drawn, not waiting for a hover').toBeGreaterThan(0.5);
    await more.tap();
    await expect(page.locator('.menu, [role="menu"]').first()).toBeVisible();
  });
}

/* a touch held on a row, as a finger does: a touch's pointerdown, nothing for 700 ms, then the menu is expected open
   before the finger lifts. iOS sends no contextmenu for a long press, so lull has to see the hold itself */
test('a touch held on a song opens its menu', async ({ page }) => {
  test.fixme(true, 'M07: only a contextmenu event opens lull\'s menu (app.js), and iOS sends none for a long press');
  await open(page, `/playlist/${ids.playlist}`);
  const row = page.locator('#view .trow[data-track-id]').nth(2);
  const b = await row.boundingBox();
  const at = { x: b.x + b.width * 0.45, y: b.y + b.height / 2 };
  await page.evaluate(({ x, y }) => {
    const t = document.elementFromPoint(x, y);
    const o = { pointerType: 'touch', pointerId: 7, isPrimary: true, clientX: x, clientY: y, bubbles: true, cancelable: true, composed: true };
    t.dispatchEvent(new PointerEvent('pointerdown', o));
    window.__lift = () => t.dispatchEvent(new PointerEvent('pointerup', o));
  }, at);
  await page.waitForTimeout(700);
  const shown = await page.locator('.menu, [role="menu"]').first().isVisible().catch(() => false);
  await page.evaluate(() => window.__lift());
  expect(shown, 'the row\'s menu, open while the finger is still down').toBe(true);
});

/* a long press on a row or tile is lull's: iOS's link preview and callout would take it */
test('a held link shows lull\'s menu, not the browser\'s callout', async ({ page, browser }) => {
  test.skip(!isWebkit(browser), '-webkit-touch-callout is WebKit\'s alone');
  test.fixme(true, 'M07: no -webkit-touch-callout: none on rows and tiles, so iOS shows its link preview on a long press');
  await open(page, '/home');
  const callout = await page.evaluate(() => [...document.querySelectorAll('#view .tile a[href], #view .trow a[href], #view .disc-pick a[href]')]
    .filter((a) => a.getClientRects().length).map((a) => getComputedStyle(a).webkitTouchCallout || getComputedStyle(a).getPropertyValue('-webkit-touch-callout')));
  expect(callout.length).toBeGreaterThan(0);
  expect(callout.filter((c) => c !== 'none'), 'links that still give iOS\'s callout').toEqual([]);
});

/* :hover sticks on a phone after a tap (iOS keeps it until the next tap elsewhere): every hover style of lull's own CSS
   (web/*.css and the screens', which every design and theme builds on) is for a pointer that can hover, inside
   @media (hover: hover). A :hover only inside :not() styles what isn't hovered, so it doesn't count. A design's and a
   listener theme's sheets are their own */
test('hover styles wait for a pointer that hovers', async ({ page }) => {
  await open(page, '/home');
  const bare = new Set();
  for (const route of ['/home', '/library', '/now', '/stats', '/settings']) {
    await go(page, route);
    for (const x of await page.evaluate(() => {
      const out = [];
      const walk = (rules, guarded, file) => {
        for (const r of rules) {
          if (r instanceof CSSMediaRule) walk(r.cssRules, guarded || /\(\s*(any-)?hover\s*:\s*hover\s*\)/.test(r.conditionText || r.media.mediaText), file);
          else if (r instanceof CSSStyleRule) {
            const sel = r.selectorText.replace(/:not\((?:[^()]|\([^()]*\))*\)/g, '');
            if (!guarded && /:hover/.test(sel) && !/scrollbar/.test(sel)) out.push(`${file}: ${r.selectorText.slice(0, 80)}`);
            if (r.cssRules) walk(r.cssRules, guarded, file);
          } else if (r.cssRules) walk(r.cssRules, guarded, file);
        }
      };
      const sheets = [];
      const collect = (s) => { sheets.push(s); for (const r of s.cssRules) if (r instanceof CSSImportRule && r.styleSheet) collect(r.styleSheet); };
      for (const s of document.styleSheets) collect(s);
      for (const s of sheets) if (s.href && !/\/(designs|packs|api)\//.test(s.href)) walk(s.cssRules, false, s.href.split('/').slice(-2).join('/'));
      return out;
    })) bare.add(x);
  }
  expect([...bare], `${bare.size} :hover rules outside @media (hover: hover)`).toEqual([]);
});

test('with reduced motion asked, nothing on Now Playing loops', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  for (const design of designs) {
    await withSong(page, design, '/now?tab=art');
    await page.evaluate(() => App.player.play());
    await page.waitForTimeout(800);
    const loops = await page.evaluate(() => document.getAnimations()
      .filter((a) => a.playState === 'running' && a.effect && a.effect.getTiming().iterations === Infinity)
      .map((a) => `${a.animationName || 'animation'} on ${a.effect.target && a.effect.target.className}`)
      .filter((s) => !/spin|sheen|xfer|load|buffer|wait/.test(s)));   // progress keeps moving, as in the loops gate
    expect(loops, design).toEqual([]);
  }
});

/* axe at a phone's width: the accessibility gate reads the screens on a desktop window, where rows and tiles are laid out
   with more room. Violations a11y-known.json lists for the desktop are not counted again */
const known = JSON.parse(readFileSync(new URL('./a11y-known.json', import.meta.url), 'utf8')).known;
for (const design of designs) {
  test(`axe finds nothing new at a phone's width / ${design}`, async ({ page, browser }) => {
    test.skip(isWebkit(browser), 'axe reads the same markup in both engines: Chromium only');
    test.fixme(['lull', 'starlight', 'glasshouse'].includes(design), 'M15: the phone\'s Now Playing seek wave is a slider with no aria-valuenow');
    const found = [];
    for (const route of ['/home', '/search?q=velvet', `/playlist/${ids.playlist}`, '/now?tab=queue']) {
      await withSong(page, design, route);
      const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
      found.push(...violations.flatMap((v) => v.nodes.map((n) => ({ route, rule: v.id, target: n.target.join(' ') })))
        .filter((x) => !known.some((k) => k.rule === x.rule && (!k.designs || k.designs.includes(design)) && new RegExp(k.target).test(x.target))));
    }
    expect(found).toEqual([]);
  });
}

/* a note on Now Playing stands clear of its transport: the page hides the mini player and tab bar the notes are placed
   above, so a note lands over the controls */
for (const design of designs) {
  test(`a note leaves Now Playing's controls free / ${design}`, async ({ page }) => {
    await withSong(page, design, '/now?tab=art');
    // while lull's disc system has no height on a phone (M03), its keys sit at the sheet's top, where the notes come down
    test.fixme(await page.evaluate(() => (document.querySelector('#view .lull-dev__screen')?.getBoundingClientRect().height ?? 100) < 100),
      'M03: lull\'s .lull-dev__screen has no height on a phone, so its transport sits under the notes at the top');
    await page.evaluate(() => { App.toast('Install lull for a full-screen player with lock-screen controls'); App.toast('Added 12 songs to the queue'); });
    await page.waitForTimeout(600);
    const covered = await page.evaluate(() => [...document.querySelectorAll('#view [data-act="prev"], #view [data-act="play"], #view [data-act="next"], #view [data-seek="player"]')]
      .filter((b) => b.getClientRects().length).filter((b) => {
        const r = b.getBoundingClientRect(), hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        return hit && hit.closest('#toasts');
      }).map((b) => b.getAttribute('data-act') || 'seek'));
    expect(covered, 'controls under a note').toEqual([]);
  });
}
