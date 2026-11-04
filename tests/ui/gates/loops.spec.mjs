// Loops drawn cheaply in every engine (docs/themes.md, Drawing cheaply): in Chromium (the desktop app on Windows, most
// browsers) and in WebKit (Linux's WebKitGTK app, Safari), every design and theme, on Home and on Now Playing with a
// song playing, each loop that never ends is stepped to the rate it names, so it draws that many frames a second
// whatever the screen's refresh rate; and while the page asks loops to hold still (:root.is-still, App.still), each
// decorative one holds and each that shows progress (--loop-holds: no) keeps going, all moving again after. perf.spec
// counts the frames themselves (Chromium's trace); this proves the mechanism holds where no trace can be read.
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, ids, openScreen, themes } from './lib.mjs';

// each running loop on the page: its rate and whether it is stepped, and its playback rate (0 = held)
const loops = (page) => page.evaluate(() => document.getAnimations()
  .filter((a) => a.effect && a.effect.target && a.effect.getTiming().iterations === Infinity && a.playState === 'running'
    && !(a.timeline && a.timeline.constructor.name !== 'DocumentTimeline'))
  .map((a) => {
    const e = a.effect, cs = getComputedStyle(e.target, e.pseudoElement), t = e.target;
    const kf = e.getKeyframes();
    return {
      name: (a.animationName || 'animation') + ' on ' + t.tagName.toLowerCase() + '.' + [...t.classList].slice(0, 2).join('.') + (e.pseudoElement || ''),
      fps: parseFloat(cs.getPropertyValue('--loop-fps')) || 0,
      holds: cs.getPropertyValue('--loop-holds').trim() !== 'no',
      stepped: /steps/.test(e.getTiming().easing) || kf.slice(0, -1).every((k) => /steps/.test(k.easing)),
      rate: a.playbackRate,
    };
  }));

for (const design of designs) {
  for (const theme of themes) {
    test(`${design} / ${theme}: every loop at its own rate, held when asked`, async ({ page }) => {
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
      await page.waitForFunction(() => App.player.state.playing, null, { timeout: 10_000 }).catch(() => {});
      for (const route of ['/home', '/now']) {
        await page.evaluate((r) => { location.hash = r; }, route);
        await page.waitForTimeout(1200);
        const now = await loops(page);
        expect(now.filter((l) => !(l.fps > 0) || !l.stepped).map((l) => l.name), `loops on ${route} that tick every frame (no --loop-fps)`).toEqual([]);
        await page.evaluate(() => App.still('gate', true));
        const held = await loops(page);
        expect(held.filter((l) => l.holds && l.rate !== 0).map((l) => l.name), `decorative loops on ${route} still moving while held`).toEqual([]);
        expect(held.filter((l) => !l.holds && l.rate === 0).map((l) => l.name), `progress loops on ${route} held`).toEqual([]);
        await page.evaluate(() => App.still('gate', false));
        const back = await loops(page);
        expect(back.filter((l) => l.rate === 0).map((l) => l.name), `loops on ${route} still held after`).toEqual([]);
        expect(back.filter((l) => !l.stepped).map((l) => l.name), `loops on ${route} unstepped after`).toEqual([]);
      }
    });
  }
}
