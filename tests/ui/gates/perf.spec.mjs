// Performance budgets, on the fixture hub with the CPU slowed 4x (a low-end phone or an old laptop). Most gates are
// counts from a Chrome trace (elements restyled, layouts, frames), which do not move between runs; the few on task
// length have room for a slow CI runner and still catch a page that blocks for a noticeable moment. The numbers and
// why they sit where they do: perf-budgets.json, docs/context/quality.md#ui-gates.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { designs, ids, openScreen, quiet, restyleFanOut, traced } from './lib.mjs';
import * as S from './scenarios.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const { rate, budgets: B } = JSON.parse(readFileSync(join(here, 'perf-budgets.json'), 'utf8'));
const opts = (design) => ({ design, theme: 'day', rate });

for (const design of designs) {
  test.describe(design, () => {
    // One small change anywhere must not restyle the whole screen (a page-wide :has() rule, a [class*=] test).
    for (const [name, route] of [['home', '/home'], ['playlist', `/playlist/${ids.playlist}`], ['now', '/now']]) {
      test(`a DOM change on ${name} restyles only what it touches`, async ({ browser, page }) => {
        await openScreen(page, route, opts(design));
        const fan = await restyleFanOut(browser, page);
        expect(fan.class, 'elements restyled by a class change').toBeLessThanOrEqual(B.fanOutElements);
        expect(fan.text, 'elements restyled by a text change').toBeLessThanOrEqual(B.fanOutElements);
        expect(fan.insert, 'elements restyled by an insertion').toBeLessThanOrEqual(B.fanOutElements);
      });
    }

    test('the hover highlight follows the pointer over a long list', async ({ browser, page }) => {
      const s = await S.hover(browser, page, opts(design));
      expect(s.landsInFrames, 'frames before the highlight is complete').toBe(0);
      // a hover only repaints; the one layout allowed is a tooltip opening where the pointer rested (on a slow runner)
      expect(s.layouts, 'layouts while hovering').toBeLessThanOrEqual(B.hoverLayouts);
      expect(s.restyled[0] ?? 0, 'most elements restyled in one hover').toBeLessThanOrEqual(B.hoverRestyleElements);
      expect(s.handlersMs, 'time in event handlers over the sweep (ms)').toBeLessThanOrEqual(B.hoverHandlersMs);
      expect(s.longestMs, 'longest task while hovering (ms)').toBeLessThanOrEqual(B.hoverLongestMs);
    });

    // paused and left alone, in both themes: the main thread idle, and the design's loops drawn cheaply (each at the rate
    // it names, on one shared beat, by the compositor, clear of live glass), so the GPU draws a few frames a second, not
    // one per refresh (a decorative loop runs on the compositor with no main-thread work, and keeps the GPU busy while
    // the listener is away)
    for (const theme of ['day', 'night']) {
      test(`little runs or draws at rest, a song paused / ${theme}`, async ({ browser, baseURL }) => {
        for (const route of ['/home', `/playlist/${ids.longPlaylist}`, '/now']) {
          const context = await browser.newContext({ baseURL, viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' }), page = await context.newPage();
          const s = await S.idle(browser, page, { ...opts(design), theme }, route);
          expect(s.frames, `main-thread frames in 2 s at rest on ${route}; in them: ${s.why}; running at the end: ${s.animations.join(' | ')}; missed the compositor at least once since load: ${s.offCompositor.join(' | ') || 'none'}`).toBeLessThanOrEqual(B.idleFrames);
          expect(s.loops, `loops drawn dearly at rest on ${route}`).toEqual([]);
          expect(s.drawn, `frames drawn in 2 s at rest on ${route}; running: ${s.animations.join(' | ')}`).toBeLessThanOrEqual(B.idleDrawnFrames);
          expect(s.composited.windows, `composited layers at rest on ${route}, in windows of area (${s.composited.layers} layers)`).toBeLessThanOrEqual(B.compositedWindows[design]);
          await context.close();
        }
      });
    }

    test('playing costs little per second', async ({ browser, page }) => {
      const s = await S.playing(browser, page, opts(design), '/now');
      expect(s.perSecondMs, 'main-thread ms per second while playing, on Now Playing').toBeLessThanOrEqual(B.playingMsPerSecond);
      expect(s.loops, 'loops drawn dearly while playing, on Now Playing').toEqual([]);
      expect(s.restyled[0] ?? 0, 'most elements restyled by one clock tick').toBeLessThanOrEqual(B.tickRestyleElements);
    });
  });
}

test.describe('lull', () => {
  const o = opts('lull');

  // "Pause animations when lull isn't the active window" (Settings › Appearance): off by default, so a window behind
  // another keeps its loops; on, the decorative loops hold still while the window has no focus (no frames drawn), a
  // spinner keeps going, and all of it moves again with focus back. The window's focus is played by document.hasFocus
  // and the window's blur and focus events, as the desktop app's webview and a browser send them
  test('decorative loops hold still while lull is not the active window, only when asked', async ({ browser, page }) => {
    await openScreen(page, '/home', { design: 'skydeck', theme: 'night' });
    await page.evaluate(() => { const s = document.createElement('span'); s.className = 'spin-ring'; s.style.cssText = 'position:fixed;left:8px;bottom:8px'; s.id = 'gate-spin'; document.body.appendChild(s); });
    await page.waitForTimeout(500);
    const focus = (on) => page.evaluate((on) => { document.hasFocus = () => on; dispatchEvent(new Event(on ? 'focus' : 'blur')); }, on);
    const state = () => page.evaluate(() => {
      const loops = document.getAnimations().filter((a) => a.effect && a.effect.getTiming().iterations === Infinity);
      const held = (sel) => loops.filter((a) => a.effect.target.matches(sel)).map((a) => a.playState);
      return { still: document.documentElement.classList.contains('is-still'), sky: held('.sky-specks'), spinner: held('#gate-spin') };
    });
    const drawn = async () => (await traced(browser, page, () => page.waitForTimeout(2000), { drawn: true })).drawn;
    const defaults = await page.evaluate(() => App.prefs().pauseInactive);
    expect(defaults, 'the setting is off by default').toBe(false);
    await focus(false);
    let st = await state();
    expect(st.still, 'off: nothing holds still while another window is active').toBe(false);
    expect(await drawn(), 'off: the loops draw while another window is active').toBeGreaterThan(10);
    await page.evaluate(() => App.setPref('pauseInactive', true));
    st = await state();
    expect(st.still, 'on: lull inactive, the loops hold still').toBe(true);
    expect(st.sky, 'on: the sky twinkle holds').toEqual(['paused']);
    expect(st.spinner, 'on: a spinner keeps going').toEqual(['running']);
    // counted once the hold and the spinner's going are on screen, drawn and the compositor quiet: what follows is lull
    // held still
    await page.evaluate(() => document.getElementById('gate-spin').remove());
    await quiet(browser, page);
    expect(await drawn(), 'on: frames drawn in 2 s while lull is inactive').toBeLessThanOrEqual(2);
    await focus(true);
    st = await state();
    expect(st.still, 'on: lull active again, nothing holds').toBe(false);
    expect(st.sky, 'on: the sky twinkles again').toEqual(['running']);
    expect(await drawn(), 'on: the loops draw again with focus back').toBeGreaterThan(10);
  });

  test('startup', async ({ browser, page }) => {
    const s = await S.startup(browser, page, o);
    expect(s.longestMs, 'longest task while starting (ms)').toBeLessThanOrEqual(B.startupLongestMs);
    expect(s.busyMs, 'main-thread ms until Home has rendered').toBeLessThanOrEqual(B.startupBusyMs);
    expect(s.domNodes, 'elements on Home').toBeLessThanOrEqual(B.homeDomNodes);
  });

  test('scrolling a long playlist', async ({ browser, page }) => {
    const s = await S.scroll(browser, page, o);
    expect(s.longestMs, 'longest task while scrolling (ms)').toBeLessThanOrEqual(B.scrollLongestMs);
    // only the rows near the screen have contents (ui.row): every row drawn was ~50k elements here
    expect(s.domNodes, 'elements on the 1,200-song playlist, scrolled part way down').toBeLessThanOrEqual(B.longPlaylistDomNodes);
  });

  // A long list costs what a short one does: every song keeps its <li> (ui.row), empty while away from the screen, and
  // that is all that grows, opened or scrolled through. Before, a 5,000-song playlist took 600 MB opened and 1.5 GB
  // scrolled through (0.27 MB a song); with only the rows near the screen drawn, still 72 KB a song scrolled through.
  test('a long playlist costs what a short one does', async ({ browser }) => {
    const short = await S.listCost(browser, o, 1200), long = await S.listCost(browser, o, 5000), more = long.songs - short.songs;
    const kb = (k) => ((long[k] - short[k]) * 1024) / more;
    console.log('list cost', JSON.stringify({ short, long }));
    expect(long.rows - short.rows, 'rows: one for each song').toBe(more);
    expect((long.elements - long.empty) - (short.elements - short.empty), 'elements besides the empty rows, 5,000 songs against 1,200').toBeLessThanOrEqual(B.listDrawnGrowth);
    expect((long.nodes - short.nodes) / more, 'DOM nodes per song more (its <li>)').toBeLessThanOrEqual(B.listNodesPerSong);
    expect(long.images - short.images, 'pictures, 5,000 songs against 1,200').toBeLessThanOrEqual(B.listImageGrowth);
    expect(kb('openedMB'), 'renderer memory per song more, opened (KB)').toBeLessThanOrEqual(B.listKBPerSongOpened);
    expect(kb('rendererMB'), 'renderer memory per song more, scrolled through (KB)').toBeLessThanOrEqual(B.listKBPerSong);
  });

  test('moving between screens', async ({ browser, page }) => {
    const s = await S.navigate(browser, page, o);
    expect(s.longestMs, 'longest task of a screen change (ms)').toBeLessThanOrEqual(B.navigateLongestMs);
  });

  test('typing a search', async ({ browser, page }) => {
    const s = await S.typeSearch(browser, page, o);
    expect(s.longestMs, 'longest task while typing (ms)').toBeLessThanOrEqual(B.typingLongestMs);
  });

  test('opening a song menu', async ({ browser, page }) => {
    const s = await S.openMenu(browser, page, o);
    expect(s.longestMs, 'longest task opening a menu (ms)').toBeLessThanOrEqual(B.menuLongestMs);
    expect(s.restyled[0] ?? 0, 'most elements restyled opening a menu').toBeLessThanOrEqual(B.menuRestyleElements);
  });

  test('dragging in the queue', async ({ browser, page }) => {
    const s = await S.queueDrag(browser, page, o);
    expect(s.longestMs, 'longest task while dragging (ms)').toBeLessThanOrEqual(B.dragLongestMs);
    expect(s.forcedLayouts, 'layouts forced by script while dragging').toBeLessThanOrEqual(B.dragForcedLayouts);
  });

  test('resizing the side panel', async ({ browser, page }) => {
    const s = await S.resizePanel(browser, page, o);
    test.skip(!s, 'no docked panel at this size');
    expect(s.longestMs, 'longest task while resizing (ms)').toBeLessThanOrEqual(B.resizeLongestMs);
  });

  test('playing beside a long list', async ({ browser, page }) => {
    const s = await S.playing(browser, page, o, `/playlist/${ids.longPlaylist}`);
    expect(s.perSecondMs, 'main-thread ms per second while playing, on a 1,200-song playlist').toBeLessThanOrEqual(B.playingMsPerSecond);
  });

  test('a long listening session leaves nothing behind', async ({ page, browser }) => {
    const m = await S.listening(browser, page, o);
    expect(m.heapGrowthMB, `JS heap growth over ${m.songs} songs and the screens between them (MB)`).toBeLessThanOrEqual(B.heapGrowthMB);
    expect(m.nodeGrowth, 'DOM nodes left behind').toBeLessThanOrEqual(B.nodeGrowth);
    expect(m.listenerGrowth, 'event listeners left behind').toBeLessThanOrEqual(B.listenerGrowth);
    // the desktop app's bridge: the cover crosses once per song, never inside the snapshot sent with every tick
    expect(m.largestPlayerSnapshot, 'largest player snapshot sent to the mini player (bytes)').toBeLessThanOrEqual(B.playerSnapshotBytes);
    expect(m.coversSent, 'covers sent to the mini player').toBeLessThanOrEqual(m.songs + 2);
    expect(m.bridgeKBPerSong, 'desktop bridge traffic per song (KB)').toBeLessThanOrEqual(B.bridgeKBPerSong);
  });
});
