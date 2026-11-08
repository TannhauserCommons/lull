// Layout switches (ADR-0101, section 4): every layout choice lull's script makes is a custom property on `.app`
// (`--layout-*`) or a fitting part (`--fit-order`, `--fit-sides`) that lull's CSS sets to what lull does today and a
// theme may set to anything else (docs/themes.md, section 5, Layout switches).
// - The five built-ins decide exactly as before: at a phone, a tablet, a short laptop, two laptops, a wide and a very
//   wide window, the attributes lull sets on `.app` at rest, with the Now column asked for, with the panel opened and
//   closed, on Now Playing's queue tab and with the search side by side, and whether the equaliser opens as a sheet or
//   a popover, are the values recorded on the code before the switches (BUILT_INS).
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

/* `.app`'s ATTRS in each design and window, in this order: at rest; the Now column asked for (and Queue and Lyrics as
   two panes); the panel opened; closed; on /now?tab=queue; then whether Search shows its two lists side by side and
   how the equaliser opens. Recorded by running this test on the code before the switches */
const BUILT_INS = {
  lull: {
    '390x844': ['closed compact tall - - - -', 'closed compact tall - - - -', 'open compact tall - - - -', 'closed compact tall - - - -', 'closed compact tall - - - -', 'split false', 'eq sheet'],
    '768x1024': ['closed regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'split false', 'eq popover'],
    '1100x700': ['closed regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'split false', 'eq popover'],
    '1366x768': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1440x900': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1920x1080': ['docked wide tall -  - -', 'docked wide tall open  - -', 'docked wide tall open  - -', 'docked wide tall open  strip -', 'docked wide tall -  - -', 'split true', 'eq popover'],
    '2560x1440': ['docked xwide tall -  - -', 'docked xwide tall open  - ', 'docked xwide tall open  - ', 'docked xwide tall open  strip ', 'docked xwide tall -  - ', 'split true', 'eq popover'],
  },
  skydeck: {
    '390x844': ['closed compact tall - - - -', 'closed compact tall - - - -', 'open compact tall - - - -', 'closed compact tall - - - -', 'closed compact tall - - - -', 'split false', 'eq sheet'],
    '768x1024': ['closed regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'split false', 'eq popover'],
    '1100x700': ['closed regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'split false', 'eq popover'],
    '1366x768': ['docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - strip -', 'docked regular tall - - - -', 'split false', 'eq popover'],
    '1440x900': ['docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - strip -', 'docked regular tall - - - -', 'split false', 'eq popover'],
    '1920x1080': ['docked wide tall -  - -', 'docked regular tall open  - -', 'docked regular tall open  - -', 'docked wide tall open  strip -', 'docked wide tall -  - -', 'split false', 'eq popover'],
    '2560x1440': ['docked xwide tall -  - -', 'docked wide tall open  - -', 'docked wide tall open  - -', 'docked xwide tall open  strip ', 'docked xwide tall -  - ', 'split true', 'eq popover'],
  },
  starlight: {
    '390x844': ['closed compact tall - - - -', 'closed compact tall - - - -', 'open compact tall - - - -', 'closed compact tall - - - -', 'closed compact tall - - - -', 'split false', 'eq sheet'],
    '768x1024': ['closed regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'split false', 'eq popover'],
    '1100x700': ['closed regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'split false', 'eq popover'],
    '1366x768': ['docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - strip -', 'docked regular tall - - - -', 'split false', 'eq popover'],
    '1440x900': ['docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - - -', 'docked regular tall - - strip -', 'docked regular tall - - - -', 'split false', 'eq popover'],
    '1920x1080': ['docked wide tall -  - -', 'docked regular tall open  - -', 'docked regular tall open  - -', 'docked wide tall open  strip -', 'docked wide tall -  - -', 'split false', 'eq popover'],
    '2560x1440': ['docked xwide tall -  - -', 'docked wide tall open  - -', 'docked wide tall open  - -', 'docked xwide tall open  strip ', 'docked xwide tall -  - ', 'split true', 'eq popover'],
  },
  petal: {
    '390x844': ['closed compact tall - - - -', 'closed compact tall - - - -', 'open compact tall - - - -', 'closed compact tall - - - -', 'closed compact tall - - - -', 'split false', 'eq sheet'],
    '768x1024': ['closed regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'split false', 'eq popover'],
    '1100x700': ['closed regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'split false', 'eq popover'],
    '1366x768': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1440x900': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1920x1080': ['docked wide tall -  - -', 'docked wide tall open  - -', 'docked wide tall open  - -', 'docked wide tall open  strip -', 'docked wide tall -  - -', 'split true', 'eq popover'],
    '2560x1440': ['docked xwide tall -  - -', 'docked xwide tall open  - ', 'docked xwide tall open  - ', 'docked xwide tall open  strip ', 'docked xwide tall -  - ', 'split true', 'eq popover'],
  },
  glasshouse: {
    '390x844': ['closed compact tall - - - -', 'closed compact tall - - - -', 'open compact tall - - - -', 'closed compact tall - - - -', 'closed compact tall - - - -', 'split false', 'eq sheet'],
    '768x1024': ['closed regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'closed regular tall - - - -', 'open regular tall - - - -', 'split false', 'eq popover'],
    '1100x700': ['closed regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'closed regular short - - - -', 'open regular short - - - -', 'split false', 'eq popover'],
    '1366x768': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1440x900': ['docked regular tall -  - -', 'docked regular tall -  - -', 'docked regular tall -  - -', 'docked wide tall -  strip -', 'docked regular tall -  - -', 'split false', 'eq popover'],
    '1920x1080': ['docked wide tall -  - -', 'docked regular tall open  - -', 'docked regular tall open  - -', 'docked wide tall open  strip -', 'docked wide tall -  - -', 'split false', 'eq popover'],
    '2560x1440': ['docked xwide tall -  - -', 'docked xwide tall open  - ', 'docked xwide tall open  - ', 'docked xwide tall open  strip ', 'docked xwide tall -  - ', 'split true', 'eq popover'],
  },
};

async function decisions(page) {
  const seen = [await attrs(page)];
  await page.evaluate(() => App.layout.set({ nowColumn: 'open', panelSplit: true })); await frames(page);
  seen.push(await attrs(page));
  await page.evaluate(() => App.openPanel('queue')); await frames(page);
  seen.push(await attrs(page));
  await page.evaluate(() => App.closePanel()); await frames(page);
  seen.push(await attrs(page));
  await page.evaluate(() => App.go('/now?tab=queue'));
  await page.waitForSelector('.now'); await frames(page);
  seen.push(await attrs(page));
  await page.evaluate(() => { App.layout.set({ searchSplit: true }); App.go('/search?q=velvet'); });
  await page.waitForSelector('.disc-search'); await page.waitForTimeout(400);
  seen.push('split ' + await page.evaluate(() => !!document.querySelector('.disc-splitview')));
  await page.evaluate(() => App.needEq().then(() => App.eq.open(document.querySelector('#pbar')))); await frames(page);
  seen.push('eq ' + await page.evaluate(() => (document.querySelector('.eq-sheet') ? 'sheet' : document.querySelector('.eq-pop') ? 'popover' : 'none')));
  return seen;
}

for (const design of designs) {
  test(`the built-ins decide as before / ${design}`, async ({ browser }) => {
    const got = {};
    for (const [width, height] of SIZES) {
      const context = await browser.newContext({ viewport: { width, height } });
      const page = await context.newPage();
      await openScreen(page, '/home', { design, clock: false });
      got[`${width}x${height}`] = await decisions(page);
      await context.close();
    }
    expect(got, 'what lull decides in this design, as before the layout switches').toEqual(BUILT_INS[design]);
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
