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
    // each probe with what it landed on, so a miss names the thing in the way
    const hits = probes.map(([px, py]) => { const hit = document.elementFromPoint(px, py); return { px, py, hit, ok: !!hit && (hit === el || el.contains(hit)) }; });
    const missed = hits.filter((h) => !h.ok);
    if (!missed.length) continue;
    let moving = 0; for (let e = el; e; e = e.parentElement) moving += e.getAnimations().length;   // a transition still running on it or above it
    out.push(`${named(el)} ${r.width.toFixed(1)}x${r.height.toFixed(1)} at ${r.left.toFixed(1)},${r.top.toFixed(1)}, ${moving} animations running on it or above it; a tap `
      + `${hits.map((h) => `${Math.round(h.px - x)},${Math.round(h.py - y)} ${h.ok ? 'reaches it' : 'misses it, hitting ' + (h.hit ? named(h.hit) : 'nothing')}`).join(' / ')}`);
  }
  if (out.length) {   // the page those controls were measured on: a page drawn zoomed out or wider than the phone shrinks them and moves every probe
    const v = window.visualViewport, d = document.documentElement;
    out.push(`page: visualViewport scale ${v.scale} (${v.width.toFixed(1)}x${v.height.toFixed(1)}), innerWidth ${innerWidth}, `
      + `documentElement.scrollWidth ${d.scrollWidth} / clientWidth ${d.clientWidth}, body.scrollWidth ${document.body.scrollWidth}, devicePixelRatio ${devicePixelRatio}`);
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
    await withSong(page, design, '/home');
    expect(await page.evaluate(tooSmall, MINI), 'under 44x44').toEqual([]);
  });

  // a listener's report (2026-11): the play key on Now Playing was squeezed to a sliver on an iPhone
  test(`Now Playing's play key is whole, round and on top / ${design}`, async ({ page }) => {
    await withSong(page, design, '/now?tab=art');
    const key = await page.evaluate((width) => {
      const b = [...document.querySelectorAll('#view [data-act="play"]')].find((e) => e.getClientRects().length);
      if (!b) return null;
      const r = b.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      // what fingers on a grid over the key reach: the key itself, not the part that clips it or something over it
      let reached = 0;
      for (const fx of [.15, .35, .5, .65, .85]) for (const fy of [.15, .35, .5, .65, .85]) {
        const at = document.elementFromPoint(r.left + r.width * fx, r.top + r.height * fy);
        if (at && b.contains(at)) reached++;
      }
      // what clips it: an ancestor that hides overflow and ends inside the key
      let cut = null;
      for (let e = b.parentElement; e; e = e.parentElement) {
        if (getComputedStyle(e).overflow === 'visible') continue;
        const c = e.getBoundingClientRect();
        if (c.top > r.top + 2 || c.bottom < r.bottom - 2 || c.left > r.left + 2 || c.right < r.right - 2) { cut = e.className; break; }
      }
      return { w: r.width, h: r.height, onTop: !!hit && b.contains(hit), cut, reached, left: r.left, right: r.right, width,
        hit: parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--hit')) };
    }, page.viewportSize().width);
    expect(key, 'a play key on Now Playing').not.toBeNull();
    expect(key.cut, 'nothing clips it').toBeNull();
    expect(key.onTop, 'a tap at its centre reaches it').toBe(true);
    expect(key.reached, 'of 25 points over the key, those that reach it (a round key has corners)').toBeGreaterThanOrEqual(17);
    expect(key.left, 'inside the screen').toBeGreaterThanOrEqual(0);
    expect(key.right, 'inside the screen').toBeLessThanOrEqual(key.width);
    expect(key.hit, '--hit, the touch target').toBeGreaterThan(0);
    expect(Math.min(key.w, key.h), 'at least 44 px, and the touch target').toBeGreaterThanOrEqual(Math.max(44, key.hit));
    expect(key.w / key.h, 'as wide as it is tall').toBeGreaterThan(0.85);
    expect(key.w / key.h).toBeLessThan(1.18);
  });

  // a listener's report (2026-11): the Search tab opened the page without putting the cursor in the field
  test(`the Search tab puts the cursor in the field / ${design}`, async ({ page }) => {
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

/* the Search tab readies the field for typing: the keyboard's stand-in gone, the words typed reaching it, and pressed
   again on search, the cursor back after the words */
test('the Search tab puts the cursor in the search field, from another page and on search itself', async ({ page }) => {
  await open(page, '/home');
  const field = page.locator('#disc-q');
  const tab = page.locator('[data-region="tabbar"] [data-nav="search"]');
  await tab.tap();
  await expect(field).toBeFocused();
  await expect(page.locator('input[aria-hidden="true"][type="search"]'), 'the stand-in for the keyboard is gone').toHaveCount(0);
  await page.keyboard.type('amber');
  await expect(field).toHaveValue('amber');
  await field.blur();
  await tab.tap();
  await expect(field, 'pressed again on search').toBeFocused();
  await expect.poll(() => page.evaluate(() => document.querySelector('#disc-q').selectionStart), 'the cursor after the words').toBe(5);
});

/* a touch held on a row, as a finger does: a touch's pointerdown, nothing for 700 ms, then the menu is expected open
   before the finger lifts. iOS sends no contextmenu for a long press, so lull has to see the hold itself */
test('a touch held on a song opens its menu', async ({ page }) => {
  await open(page, `/playlist/${ids.playlist}`);
  const row = page.locator('#view .trow[data-track-id]').nth(2);
  await row.evaluate((el) => el.scrollIntoView({ block: 'center' }));   // clear of the player bar and tab bar
  const b = await row.boundingBox();
  const at = { x: b.x + b.width * 0.45, y: b.y + b.height / 2 };
  await page.evaluate(({ x, y }) => {
    const t = document.elementFromPoint(x, y);
    if (!t.closest('.trow')) throw new Error('the row is covered by ' + t.className);
    const o = { pointerType: 'touch', pointerId: 7, isPrimary: true, clientX: x, clientY: y, bubbles: true, cancelable: true, composed: true };
    t.dispatchEvent(new PointerEvent('pointerdown', o));
    window.__lift = () => t.dispatchEvent(new PointerEvent('pointerup', o));
  }, at);
  await page.waitForTimeout(700);
  const shown = await page.locator('.menu, [role="menu"]').first().isVisible().catch(() => false);
  await page.evaluate(() => window.__lift());
  expect(shown, 'the row\'s menu, open while the finger is still down').toBe(true);
});

/* the hold is not a scroll or a tap: a finger that moves past a few pixels (a scroll, or a drag of the row) or lifts early
   opens nothing */
for (const how of ['moves', 'lifts early']) {
  test(`a touch that ${how} on a song opens no menu`, async ({ page }) => {
    await open(page, `/playlist/${ids.playlist}`);
    const row = page.locator('#view .trow[data-track-id]').nth(2);
    await row.evaluate((el) => el.scrollIntoView({ block: 'center' }));
    const b = await row.boundingBox();
    await page.evaluate(({ x, y, how }) => {
      const t = document.elementFromPoint(x, y);
      const o = (dx, dy) => ({ pointerType: 'touch', pointerId: 7, isPrimary: true, clientX: x + dx, clientY: y + dy, bubbles: true, cancelable: true, composed: true });
      t.dispatchEvent(new PointerEvent('pointerdown', o(0, 0)));
      if (how === 'moves') setTimeout(() => t.dispatchEvent(new PointerEvent('pointermove', o(0, 24))), 150);
      else setTimeout(() => t.dispatchEvent(new PointerEvent('pointerup', o(0, 0))), 150);
    }, { x: b.x + b.width * 0.45, y: b.y + b.height / 2, how });
    await page.waitForTimeout(800);
    await expect(page.locator('.menu, [role="menu"]')).toHaveCount(0);
  });
}

/* a long press on a row or tile is lull's: iOS's link preview and callout would take it */
test('a held link shows lull\'s menu, not the browser\'s callout', async ({ page, browser }) => {
  test.skip(!isWebkit(browser), '-webkit-touch-callout is WebKit\'s alone');
  await open(page, '/home');
  /* Playwright's WebKit on Windows doesn't know the property (iOS's alone: CSS.supports is false and the computed value
     empty), so this reads the declarations out of the style sheets: a link counts when a rule that sets it to none matches */
  const callout = await page.evaluate(async () => {
    const css = await Promise.all([...document.styleSheets].filter((s) => s.href).map((s) => fetch(s.href).then((r) => r.text())));
    const none = [];
    for (const m of css.join('\n').replace(/\/\*[\s\S]*?\*\//g, '').matchAll(/([^{}]+)\{[^{}]*-webkit-touch-callout\s*:\s*none[^{}]*\}/g)) none.push(m[1].trim());
    return [...document.querySelectorAll('#view .tile a[href], #view .trow a[href], #view .disc-pick a[href]')]
      .filter((a) => a.getClientRects().length).map((a) => none.some((sel) => a.matches(sel)) ? 'none' : 'default');
  });
  expect(callout.length).toBeGreaterThan(0);
  expect(callout.filter((c) => c !== 'none'), 'links that still give iOS\'s callout').toEqual([]);
});

/* :hover sticks on a phone after a tap (iOS keeps it until the next tap elsewhere): every hover style of lull's own CSS
   (web/*.css and the screens', which every design and theme builds on) and of each built-in design's theme.css is for a
   pointer that can hover, inside @media (hover: hover). A :hover only inside :not() styles what isn't hovered, so it
   doesn't count. A listener theme's sheet is its own */
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
  for (const design of designs) {
    for (const x of await page.evaluate(async (d) => {
      const css = await (await fetch(`/designs/${d}/theme.css`)).text();
      const sheet = new CSSStyleSheet(); sheet.replaceSync(css.replace(/@import[^;]*;/g, ''));
      const out = [];
      const walk = (rules, guarded) => {
        for (const r of rules) {
          if (r instanceof CSSMediaRule) walk(r.cssRules, guarded || /\(\s*(any-)?hover\s*:\s*hover\s*\)/.test(r.conditionText || r.media.mediaText));
          else if (r instanceof CSSStyleRule) {
            const sel = r.selectorText.replace(/:not\((?:[^()]|\([^()]*\))*\)/g, '');
            if (!guarded && /:hover/.test(sel) && !/scrollbar/.test(sel)) out.push(`designs/${d}/theme.css: ${r.selectorText.slice(0, 80)}`);
          } else if (r.cssRules) walk(r.cssRules, guarded);
        }
      };
      walk(sheet.cssRules, false);
      return out;
    }, design)) bare.add(x);
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

/* a touch held on a heart opens the playlist picker for its song, before the finger lifts, and the lift likes
   nothing */
test('a touch held on a heart opens the playlist picker and likes nothing', async ({ page }) => {
  await open(page, `/playlist/${ids.playlist}`);
  const id = await page.locator('#view .trow[data-track-id] [data-act="like"][aria-pressed="false"]').nth(1).getAttribute('data-id');
  const heart = page.locator(`#view [data-act="like"][data-id="${id}"]`);
  await heart.evaluate((el) => el.scrollIntoView({ block: 'center' }));
  const b = await heart.boundingBox();
  await page.evaluate(({ x, y }) => {
    const t = document.elementFromPoint(x, y);
    if (!t.closest('[data-act="like"]')) throw new Error('the heart is covered by ' + t.className);
    const o = { pointerType: 'touch', pointerId: 7, isPrimary: true, clientX: x, clientY: y, bubbles: true, cancelable: true, composed: true };
    t.dispatchEvent(new PointerEvent('pointerdown', o));
    window.__lift = () => { t.dispatchEvent(new PointerEvent('pointerup', o)); t.dispatchEvent(new MouseEvent('click', o)); };
  }, { x: b.x + b.width / 2, y: b.y + b.height / 2 });
  await page.waitForTimeout(700);
  const shown = await page.locator('.lib-picker').isVisible().catch(() => false);
  await page.evaluate(() => window.__lift());
  expect(shown, 'the picker, open while the finger is still down').toBe(true);
  await expect(page.locator('.lib-picker.is-open')).toBeVisible();
  await expect(heart, 'the lift liked nothing').toHaveAttribute('aria-pressed', 'false');
});

/* a tap on a heart likes as ever, and the first like by touch on the device says the heart can be held, once (a held
   heart has no tooltip on a phone to say it) */
test('the first like by touch says a heart can be held, once', async ({ page }) => {
  // its likes are its own (X-Fixture-Client), so the songs it likes stay unliked for every other gate
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` });
  await open(page, `/playlist/${ids.playlist}`);
  const unliked = page.locator('#view .trow[data-track-id] [data-act="like"][aria-pressed="false"]');
  const like = async (n) => {
    const heart = page.locator(`#view [data-act="like"][data-id="${await unliked.nth(n).getAttribute('data-id')}"]`);
    await heart.evaluate((el) => el.scrollIntoView({ block: 'center' }));
    await heart.tap();
    await expect(heart).toHaveAttribute('aria-pressed', 'true');
  };
  const hinted = page.locator('.toast', { hasText: 'Added to Liked songs. Hold ♥ to add a song to a playlist' });
  await like(1);
  await expect(hinted).toBeVisible();
  // a second like straight after: a plain toast of its own, and the hint still says what it says
  await like(0);
  await expect(page.locator('.toast', { hasText: /^Added to Liked songs\s*Undo/ })).toBeVisible();
  await expect(hinted).toBeVisible();
  await expect(page.locator('.toast', { hasText: 'Hold ♥' })).toHaveCount(1);
});
