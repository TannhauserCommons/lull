// Performance budgets, on the fixture hub with the CPU slowed 4x (a low-end phone or an old laptop). Most gates are
// counts from a Chrome trace (elements restyled, layouts, frames), which do not move between runs; the few on task
// length have room for a slow CI runner and still catch a page that blocks for a noticeable moment. The numbers and
// why they sit where they do: perf-budgets.json, docs/context/quality.md#ui-gates.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { designs, ids, openScreen, restyleFanOut } from './lib.mjs';
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

    // paused and left alone: no frames at all, the compositor's own too (a decorative loop runs there with no main-thread
    // work, and keeps the GPU busy while the listener is away)
    test('nothing runs or draws at rest, a song paused', async ({ browser, baseURL }) => {
      for (const route of ['/home', `/playlist/${ids.longPlaylist}`, '/now']) {
        const context = await browser.newContext({ baseURL, viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' }), page = await context.newPage();
        const s = await S.idle(browser, page, opts(design), route);
        expect(s.frames, `main-thread frames in 2 s at rest on ${route}; running: ${s.animations.join(' | ')}`).toBeLessThanOrEqual(B.idleFrames);
        expect(s.drawn, `frames drawn in 2 s at rest on ${route}; running: ${s.animations.join(' | ')}`).toBeLessThanOrEqual(B.idleDrawnFrames);
        await context.close();
      }
    });

    test('playing costs little per second', async ({ browser, page }) => {
      const s = await S.playing(browser, page, opts(design), '/now');
      expect(s.perSecondMs, 'main-thread ms per second while playing, on Now Playing').toBeLessThanOrEqual(B.playingMsPerSecond);
      expect(s.restyled[0] ?? 0, 'most elements restyled by one clock tick').toBeLessThanOrEqual(B.tickRestyleElements);
    });
  });
}

test.describe('lull', () => {
  const o = opts('lull');

  test('startup', async ({ browser, page }) => {
    const s = await S.startup(browser, page, o);
    expect(s.longestMs, 'longest task while starting (ms)').toBeLessThanOrEqual(B.startupLongestMs);
    expect(s.busyMs, 'main-thread ms until Home has rendered').toBeLessThanOrEqual(B.startupBusyMs);
    expect(s.domNodes, 'elements on Home').toBeLessThanOrEqual(B.homeDomNodes);
  });

  test('scrolling a long playlist', async ({ browser, page }) => {
    const s = await S.scroll(browser, page, o);
    expect(s.longestMs, 'longest task while scrolling (ms)').toBeLessThanOrEqual(B.scrollLongestMs);
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
