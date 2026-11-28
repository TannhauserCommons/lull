// Layout switches (ADR-0101, section 4): every layout choice lull's script makes is a custom property on `.app`
// (`--layout-*`) or a fitting part (`--fit-order`, `--fit-sides`) that lull's CSS sets to what lull does today and a
// theme may set to anything else (docs/themes.md, section 5, Layout switches).
// - The five built-ins lay out as they do: at a phone, a tablet, a short laptop, two laptops, a wide and a very wide
//   window, what a person sees of each layout choice (the nav, the side panel docked or over the page, the Now column,
//   two panes, Search side by side, the equaliser as a sheet or a popover) is what BUILT_INS says, read from the boxes
//   on the page. A failure names the design and the window.
// - A fixture theme that sets switches gets what it set: the side panel docked along the bottom at 1100 px (where lull
//   opens it over the page) and dragged by its height, the Now column kept open, Like and Save the last to give way
//   in the song's side, and a one-capsule player bar with no mirrored sides.

import { expect, test } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { chrome } from '../checks.mjs';
import { designs, ids, openScreen, ownHub } from './lib.mjs';

const frames = (page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
const ATTRS = ['data-ctx', 'data-size', 'data-height', 'data-nowcol', 'data-nowcol-ok', 'data-panel', 'data-panel-split'];
const attrs = (page) => page.evaluate((names) => names.map((n) => document.getElementById('app').getAttribute(n) ?? '-').join(' '), ATTRS);

const SIZES = [[390, 844], [768, 1024], [1100, 700], [1366, 768], [1440, 900], [1920, 1080], [2560, 1440]];

/* What a person sees lull do in each built-in design, at each of SIZES in order: where the nav is; where the side panel
   (queue, lyrics, activity) is at rest, opened and closed again; whether the Now Playing column's button shows, and
   where the column is once asked for and once the panel opens beside it; where the queue shows on /now?tab=queue;
   whether Search shows its two lists side by side and Queue and Lyrics as two panes, each asked for; and how the
   equaliser opens; and at rest, the size class and window height a theme styles by (.app[data-size], [data-height]:
   docs/themes.md, Layout switches). ALL is what lull's own CSS does in every design; a design's own rows follow. An
   intended change to a layout edits the cells it changes */
const hidden = 'hidden', beside = 'beside the page', over = 'over the page', strip = 'a strip beside the page';
const ALL = {
  'size class a theme styles by': ['compact', 'regular', 'regular', 'regular', 'regular', 'wide', 'xwide'],
  'window height a theme styles by': ['tall', 'tall', 'short', 'tall', 'tall', 'tall', 'tall'],
  'panel at rest': [hidden, hidden, hidden, beside, beside, beside, beside],
  'Now column asked for': [hidden, hidden, hidden, hidden, hidden, beside, beside],
  'panel opened': [over, over, over, beside, beside, beside, beside],
  'Now column beside the open panel': [hidden, hidden, hidden, hidden, hidden, beside, beside],
  'panel closed': [hidden, hidden, hidden, strip, strip, strip, strip],
  'queue on /now?tab=queue': ['Now Playing\'s queue tab', 'the panel, over the page', 'the panel, over the page', 'the panel, beside the page',
    'the panel, beside the page', 'the panel, beside the page', 'the panel, beside the page'],
  equaliser: ['a sheet', 'a popover', 'a popover', 'a popover', 'a popover', 'a popover', 'a popover'],
  'Queue and Lyrics asked as two panes': ['one pane', 'one pane', 'one pane', 'one pane', 'one pane', 'one pane', 'two panes'],
};
/* where the designs differ: the nav, from which window the Now column is offered, and from which Search's two lists fit */
const header = ['a tab bar', 'the header', 'the header', 'the header', 'the header', 'the header', 'the header'];
const rail = ['a tab bar', 'a rail', 'a rail', 'a rail', 'a rail', 'a rail', 'a rail'];
const nowFrom1366 = ['none', 'none', 'none', 'shown', 'shown', 'shown', 'shown'];
const nowFrom1920 = ['none', 'none', 'none', 'none', 'none', 'shown', 'shown'];
const listsFrom1920 = ['one list', 'one list', 'one list', 'one list', 'one list', 'side by side', 'side by side'];
const listsFrom2560 = ['one list', 'one list', 'one list', 'one list', 'one list', 'one list', 'side by side'];
const BUILT_INS = {
  lull: { nav: header, ...ALL, 'Now column button': nowFrom1366, 'Search asked side by side': listsFrom1920 },
  skydeck: { nav: rail, ...ALL, 'Now column button': nowFrom1920, 'Search asked side by side': listsFrom2560 },
  starlight: { nav: rail, ...ALL, 'Now column button': nowFrom1920, 'Search asked side by side': listsFrom2560 },
  petal: { nav: header, ...ALL, 'Now column button': nowFrom1366, 'Search asked side by side': listsFrom1920 },
  // its rail lies along the bottom as a dock at a tablet's width
  glasshouse: { nav: ['a tab bar', 'a dock', 'a rail', 'a rail', 'a rail', 'a rail', 'a rail'], ...ALL, 'Now column button': nowFrom1366,
    'Search asked side by side': listsFrom2560 },
};

/* what moves the parts measured has come to rest: the transitions and animations running on them or around them (a
   panel sliding in, a sheet rising), a design's endless loops aside */
const settled = (page) => page.evaluate(() => {
  const parts = ['#ctx', '#nowcol', '.eq-sheet', '.eq-pop'].map((q) => document.querySelector(q)).filter(Boolean);
  return Promise.all(document.getAnimations().filter((a) => a.playState === 'running' && a.effect.getTiming().iterations !== Infinity
    && parts.some((p) => a.effect.target && a.effect.target.contains(p))).map((a) => a.finished.catch(() => {})));
});
/* where each part stands, by its box against the main area's */
const seen = (page) => page.evaluate(() => {
  const box = (el) => {
    if (!el) return null;
    for (let e = el; e; e = e.parentElement) {
      const s = getComputedStyle(e);
      if (s.display === 'none' || s.visibility === 'hidden' || +s.opacity === 0) return null;
    }
    const r = el.getBoundingClientRect();
    return r.width > 1 && r.height > 1 && r.right > 0 && r.left < innerWidth && r.bottom > 0 && r.top < innerHeight ? r : null;
  };
  const main = document.querySelector('.main').getBoundingClientRect();
  const over = (r) => r.left < main.right - 1 && r.right > main.left + 1 && r.top < main.bottom - 1 && r.bottom > main.top + 1;
  const strip = parseFloat(getComputedStyle(document.getElementById('app')).getPropertyValue('--layout-strip'));
  /* over the page, beside it, folded to its strip beside it, or not shown */
  const place = (el) => {
    const r = box(el);
    return !r ? 'hidden' : over(r) ? 'over the page' : r.width <= strip + 2 ? 'a strip beside the page' : 'beside the page';
  };
  const rail = box(document.querySelector('.rail')), tabs = box(document.querySelector('.tabbar'));
  const cols = [...document.querySelectorAll('.disc-results .disc-col')].map((c) => c.getBoundingClientRect());
  const eq = document.querySelector('.eq-sheet, .eq-pop')?.getBoundingClientRect();
  return {
    nav: tabs ? 'a tab bar' : rail ? (rail.height > rail.width ? 'a rail' : 'a dock')
      : [...document.querySelectorAll('.header [data-nav]')].some(box) ? 'the header' : 'none',
    size: document.getElementById('app').getAttribute('data-size'),
    height: document.getElementById('app').getAttribute('data-height'),
    panel: place(document.getElementById('ctx')),
    twoPanes: !!box(document.getElementById('ctx-body')) && !!box(document.getElementById('ctx-body-2')),
    nowColumn: place(document.getElementById('nowcol')),
    nowButton: !!box(document.querySelector('.pbar__nowcol')),
    nowQueueTab: !!box(document.querySelector('#now-tab-queue[aria-selected="true"]')),
    search: cols.length === 2 && cols[1].left >= cols[0].right - 1 && cols[1].top < cols[0].bottom ? 'side by side' : 'one list',
    eq: !eq ? 'none' : eq.width >= innerWidth * 0.9 && Math.abs(eq.bottom - innerHeight) <= 2 ? 'a sheet' : 'a popover',
  };
});
const step = async (page) => { await frames(page); await settled(page); return seen(page); };

async function outcomes(page) {
  const rest = await step(page);
  await page.evaluate(() => App.layout.set({ nowColumn: 'open', panelSplit: true }));
  const asked = await step(page);
  await page.evaluate(() => App.openPanel('queue'));
  const opened = await step(page);
  await page.evaluate(() => App.closePanel());
  const closed = await step(page);
  await page.evaluate(() => App.go('/now?tab=queue'));
  await page.waitForSelector('.now');
  const now = await step(page);
  await page.evaluate(() => { App.layout.set({ searchSplit: true }); App.go('/search?q=velvet'); });
  await page.waitForSelector('.disc-results .disc-col:not(.is-stale)');
  const search = await step(page);
  await page.evaluate(() => App.needEq().then(() => App.eq.open(document.querySelector('#pbar'))));
  const eq = await step(page);
  /* Queue and Lyrics as two panes, asked for while a song with lyrics is loaded and nothing else shows them (the Now
     column would, and the panel then leaves Lyrics out) */
  await page.keyboard.press('Escape');
  const lyrics = page.waitForResponse((r) => r.url().includes('/api/lyrics/'));
  await page.evaluate(async (album) => {
    App.layout.set({ nowColumn: false });
    App.go('/home');
    const { album: a } = await (await fetch('/api/albums/' + album)).json();
    await App.player.playList([a.trackIds[0]], 0);
    App.player.pause();
  }, ids.album);
  await lyrics;
  await page.evaluate(() => App.openPanel('queue'));
  // the panel has taken the lyrics in, as their own tab or as the second pane
  await page.waitForSelector('#ctx-tab-lyrics, #ctx-body-2:not([hidden])', { state: 'attached' });
  const panes = await step(page);
  return {
    nav: rest.nav,
    'size class a theme styles by': rest.size,
    'window height a theme styles by': rest.height,
    'panel at rest': rest.panel,
    'Now column button': rest.nowButton ? 'shown' : 'none',
    'Now column asked for': asked.nowColumn,
    'panel opened': opened.panel,
    'Now column beside the open panel': opened.nowColumn,
    'panel closed': closed.panel,
    'queue on /now?tab=queue': now.nowQueueTab && now.panel === 'hidden' ? 'Now Playing\'s queue tab' : `the panel, ${now.panel}`,
    'Search asked side by side': search.search,
    equaliser: eq.eq,
    'Queue and Lyrics asked as two panes': panes.twoPanes ? 'two panes' : 'one pane',
  };
}

for (const design of designs) {
  SIZES.forEach(([width, height], i) => {
    test(`the built-ins lay out as they do / ${design} at ${width}x${height}`, async ({ browser }) => {
      const context = await browser.newContext({ viewport: { width, height } });
      const page = await context.newPage();
      await openScreen(page, '/home', { design, clock: false });
      const got = await outcomes(page);
      await context.close();
      const want = Object.fromEntries(Object.entries(BUILT_INS[design]).map(([what, at]) => [what, at[i]]));
      expect(got, `what a person sees in ${design} at ${width}x${height}`).toEqual(want);
    });
  });
}

/* two fixture themes, each setting switches as a listener's theme would */
const THEMES = {
  // the side panel docked along the bottom from 1000 px, by its height; the Now column open on every screen but Now
  // Playing, and offered at every width (never closed for room) under 1200 px; Like and Save the last to give way in
  // the song's side, after its chips (the chips' row handed to the song's side with display: contents)
  switches: `
    @media (min-width: 1000px) {
      .app { --layout-ctx: docked; --layout-ctx-axis: block; }
      .app[data-ctx="docked"] {
        grid-template-columns: var(--rail) minmax(0, 1fr);
        grid-template-rows: auto auto minmax(0, 1fr) var(--ctx-h-drag, var(--ctx-h)) auto;
        grid-template-areas: "rail head" "rail banner" "rail main" "rail ctx" "bar bar";
      }
      .app[data-ctx="docked"] .ctx { width: auto; }
    }
    .app { --layout-nowcol: open; }
    @media (max-width: 1199px) { .app { --layout-nowcol: kept; } }
    .pbar__badges { display: contents; }
    .pbar__acts { --fit-order: 9; }`,
  // the panel's own drag bounds and popovers that open above their key (ADR-0101 amended, E: no layout number in script)
  bounds: `.app { --layout-panel-min: 320px; --layout-panel-max: 400px; --layout-popover-side: above; --layout-popover-margin: 24px; }`,
  // one capsule in the middle of the window, its parts side by side: lull's mirrored sides end
  capsule: `
    .pbar { --fit-sides: none; justify-self: center; width: min(100%, 980px); grid-template-columns: minmax(0, 1fr) auto auto; }`,
};
let hub, dir;
test.describe('a theme that sets switches', () => {
  test.beforeAll(async () => {
    dir = mkdtempSync(join(tmpdir(), 'lull-layout-switches-'));
    for (const [id, css] of Object.entries(THEMES)) {
      mkdirSync(join(dir, id), { recursive: true });
      writeFileSync(join(dir, id, 'theme.json'), JSON.stringify({ id, name: id, api: 1, extends: null, shell: 'rail', swatches: ['#141516', '#d6d3c9', '#ffa53d', '#ffb24a'], modes: ['day', 'night'] }));
      writeFileSync(join(dir, id, 'theme.css'), css);
    }
    hub = await ownHub({ FIXTURE_THEMES: dir });
  });
  test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

  async function open(browser, theme, viewport, layout) {
    const context = await browser.newContext({ baseURL: hub.url, viewport });
    const page = await context.newPage();
    if (layout) await page.addInitScript((l) => localStorage.setItem('lull:layout', JSON.stringify(l)), layout);
    await openScreen(page, '/home', { design: theme, clock: false });
    return { page, close: () => context.close() };
  }
  async function playOne(page) {
    await page.evaluate((id) => { location.hash = '#/album/' + id; }, ids.album);
    await page.dblclick('.tlist .trow:nth-child(1) .trow__dur');
    await page.waitForFunction(() => App.player.current());
    await page.evaluate(() => { App.player.pause(); location.hash = '#/home'; });
    await page.waitForTimeout(500);
  }

  test('the side panel docks along the bottom at 1100 px and drags by its height', async ({ browser }) => {
    const { page, close } = await open(browser, 'switches', { width: 1100, height: 800 });
    await page.evaluate(() => App.openPanel('queue')); await frames(page);
    expect(await page.getAttribute('#app', 'data-ctx'), 'docked where lull opens it over the page').toBe('docked');
    const box = () => page.evaluate(() => {
      const m = document.querySelector('.main').getBoundingClientRect(), c = document.getElementById('ctx').getBoundingClientRect();
      return { mainBottom: Math.round(m.bottom), ctxTop: Math.round(c.top), ctxH: Math.round(c.height), ctxW: Math.round(c.width), mainW: Math.round(m.width) };
    });
    const before = await box();
    expect(before.ctxTop, 'the panel lies under the main area').toBeGreaterThanOrEqual(before.mainBottom);
    expect(before.ctxW, 'as wide as the main area').toBeGreaterThanOrEqual(before.mainW);
    const handle = page.locator('#ctx [data-resize="panel"]');
    await expect(handle, 'its handle drags its height').toHaveAttribute('aria-orientation', 'horizontal');
    const h = await handle.boundingBox();
    expect(h.width, 'the handle lies along the panel\'s top edge').toBeGreaterThan(h.height);
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
    await page.mouse.down();
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2 - 80, { steps: 8 });
    await page.mouse.up(); await frames(page);
    expect(await page.evaluate(() => App.layout.get().panelHeight), 'dragged up by 80 px from the token\'s 320 px').toBe(400);
    expect((await box()).ctxH - before.ctxH, 'the panel grew by as much').toBe(80);
    await handle.focus(); await page.keyboard.press('ArrowDown'); await frames(page);
    expect(await page.evaluate(() => App.layout.get().panelHeight), 'the down arrow makes it shorter').toBe(384);
    expect(await page.evaluate(() => App.layout.get().panelWidth), 'its width is left as it was').toBe(null);
    await close();
  });

  test('the panel drags within the theme\'s bounds, and a popover opens on the theme\'s side, its margin from the edge', async ({ browser }) => {
    const { page, close } = await open(browser, 'bounds', { width: 1440, height: 900 });
    await page.evaluate(() => App.openPanel('queue')); await frames(page);
    const handle = page.locator('#ctx [data-resize="panel"]');
    await expect(handle).toHaveAttribute('aria-valuemin', '320');
    await expect(handle).toHaveAttribute('aria-valuemax', '400');
    await handle.focus(); await handle.press('End'); await frames(page);
    expect(await page.evaluate(() => App.layout.get().panelWidth), 'End: the theme\'s widest').toBe(400);
    await handle.press('Home'); await frames(page);
    expect(await page.evaluate(() => App.layout.get().panelWidth), 'Home: the theme\'s narrowest').toBe(320);
    const placed = await page.evaluate(() => {
      const a = document.createElement('button'); a.textContent = 'anchor';
      Object.assign(a.style, { position: 'fixed', left: '4px', top: '400px' });
      document.body.appendChild(a);
      const pop = App.popover(a, '<p style="margin:0;width:120px;height:60px">menu</p>', { label: 'Test' });
      const r = pop.el.getBoundingClientRect(), k = a.getBoundingClientRect();
      pop.close();
      return { above: r.bottom <= k.top, left: Math.round(r.left) };
    });
    expect(placed.above, 'above its key, with room there').toBe(true);
    expect(placed.left, 'kept 24 px from the window\'s edge').toBe(24);
    await close();
  });

  test('the Now column stays open: offered at any width, never closed for room, open on every screen but Now Playing', async ({ browser }) => {
    let { page, close } = await open(browser, 'switches', { width: 1440, height: 900 });
    expect(await attrs(page), 'open with nothing asked, and no button to close it').toBe('docked regular tall open - - -');
    await page.evaluate(() => App.go('/now')); await page.waitForSelector('.now'); await frames(page);
    expect(await page.getAttribute('#app', 'data-nowcol'), 'Now Playing shows it all already').toBe(null);
    await page.evaluate(() => App.go('/home')); await page.waitForSelector('.disc-home'); await frames(page);
    expect(await page.getAttribute('#app', 'data-nowcol')).toBe('open');
    await close();
    // kept: at 1100 px, where lull doesn't offer it, the listener's column opens, and the panel opening leaves it open
    ({ page, close } = await open(browser, 'switches', { width: 1100, height: 800 }, { nowColumn: 'open' }));
    expect(await attrs(page), 'offered and open at 1100 px, beside a main area that is compact now').toBe('docked compact tall open  - -');
    await page.evaluate(() => { App.closePanel(); App.openPanel('queue'); }); await frames(page);
    expect(await attrs(page), 'the panel opens again, and the column stays').toBe('docked compact tall open  - -');
    expect(await page.evaluate(() => App.layout.get().nowColumn), 'and the listener\'s choice is kept').toBe('open');
    await close();
  });

  test('Like and Save are the last to give way in the song\'s side (--fit-order)', async ({ browser }) => {
    const { page, close } = await open(browser, 'switches', { width: 1440, height: 900 });
    await playOne(page);
    let before = 0;
    for (let w = 1440; w >= 700; w -= 20) {
      await page.setViewportSize({ width: w, height: 900 }); await frames(page);
      const side = await page.evaluate(() => {
        const row = document.getElementById('pbar-now'), acts = row.querySelector('.pbar__acts');
        const others = [...row.querySelectorAll('[data-fit]')].filter((p) => p !== acts && p.closest('#pbar-now') === row && !p.closest('.pbar__acts'));
        return { acts: acts.hasAttribute('data-fit-out'), out: others.filter((p) => p.hasAttribute('data-fit-out')).length, all: others.length, gave: row.getAttribute('data-fit-gave') };
      });
      if (side.acts) expect(side, `${w} px: Like and Save gave way only after every chip`).toEqual({ acts: true, out: side.all, all: side.all, gave: expect.stringContaining('9') });
      else if (side.out) before++;
    }
    expect(before, 'widths at which a chip gave way and Like and Save stayed').toBeGreaterThan(0);
    await close();
  });

  test('a one-capsule player bar: lull\'s mirrored sides end (--fit-sides: none), and the bar still works', async ({ browser }) => {
    const { page, close } = await open(browser, 'capsule', { width: 1440, height: 900 });
    await playOne(page);
    for (const w of [1440, 1100, 800]) {
      await page.setViewportSize({ width: w, height: 900 }); await frames(page);
      // the panel lull docked at 1440 px slides away under 1360: checked once it has gone
      await page.waitForFunction(() => document.getElementById('app').getAttribute('data-ctx') === 'docked' || getComputedStyle(document.getElementById('ctx')).visibility === 'hidden');
      const bar = await page.evaluate(() => {
        const b = document.getElementById('pbar'), r = b.getBoundingClientRect();
        return { side: b.style.getPropertyValue('--fit-side'), left: Math.round(r.left), right: Math.round(innerWidth - r.right) };
      });
      expect(bar.side, `${w} px: lull sizes no mirrored sides`).toBe('');
      expect(Math.abs(bar.left - bar.right), `${w} px: the capsule stands in the middle`).toBeLessThanOrEqual(1);
      expect(await chrome(page), `${w} px: every control in the header and the bar can be clicked, every text read`).toEqual([]);
    }
    await close();
  });
});
