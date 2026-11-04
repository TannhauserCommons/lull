// Loops drawn cheaply in every engine (docs/themes.md, Drawing cheaply): in Chromium (the desktop app on Windows, most
// browsers) and in WebKit (Linux's WebKitGTK app, Safari), every design and theme, on Home and on Now Playing with a
// song playing, each loop that never ends changes at most 30 times a second, stepped where it is declared, from its
// first frame, whatever the screen's refresh rate; and while the page asks loops to hold still (:root.is-still,
// App.still), each decorative one is paused and only those that show progress (lull's spinners, loading sheens and
// transfer edges) keep going, all moving again after. A theme's loop declared the older way (--loop-fps, --loop-holds:
// no) is stepped to its rate and held by lull, so themes written to that contract keep working. perf.spec counts the
// frames themselves (Chromium's trace); this proves the property where no trace can be read.
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { designs, ids, loopRate, openScreen, themes } from './lib.mjs';

// the parts that show something happening, whose loops keep going while the page holds still (components.css)
const PROGRESS = '.spin-ring, .btn-gloss__spin, .skel, .seek, .progress--xfer';

// each loop on the page that never ends: its changes a second, whether it shows progress, and whether it is paused
const loops = (page) => page.evaluate(([src, progress]) => {
  const rate = (0, eval)('(' + src + ')');
  return document.getAnimations()
    .filter((a) => a.effect && a.effect.target && a.effect.getTiming().iterations === Infinity && a.playState !== 'idle' && a.playState !== 'finished'
      && !(a.timeline && a.timeline.constructor.name !== 'DocumentTimeline'))
    .map((a) => {
      const t = a.effect.target;
      return {
        name: (a.animationName || 'animation') + ' on ' + t.tagName.toLowerCase() + '.' + [...t.classList].slice(0, 2).join('.') + (a.effect.pseudoElement || ''),
        perSecond: rate(a),
        progress: !!t.closest(progress),
        paused: a.playState === 'paused' || a.playbackRate === 0,   // held by its CSS, or (a theme's --loop-fps loop) by lull
      };
    });
}, [loopRate.toString(), PROGRESS]);
const named = (ls) => ls.map((l) => `${l.name} (${l.perSecond === Infinity ? 'every frame' : Math.round(l.perSecond) + ' a second'})`);

for (const design of designs) {
  for (const theme of themes) {
    test(`${design} / ${theme}: every loop stepped from its first frame, held when asked`, async ({ page }) => {
      await openScreen(page, `/album/${ids.album}`, { design, theme });
      await page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));
      await page.waitForFunction(() => App.player.state.playing, null, { timeout: 10_000 }).catch(() => {});
      for (const route of ['/home', '/now']) {
        await page.evaluate((r) => { location.hash = r; }, route);
        await page.waitForTimeout(1200);
        const now = await loops(page);
        expect(named(now.filter((l) => l.perSecond > 30.5)), `loops on ${route} that change more than 30 times a second`).toEqual([]);
        await page.evaluate(() => App.still('gate', true));
        const held = await loops(page);
        expect(named(held.filter((l) => !l.progress && !l.paused)), `decorative loops on ${route} still moving while held`).toEqual([]);
        expect(named(held.filter((l) => l.progress && l.paused)), `progress loops on ${route} held`).toEqual([]);
        await page.evaluate(() => App.still('gate', false));
        const back = await loops(page);
        expect(named(back.filter((l) => l.paused && !l.progress)), `loops on ${route} still held after`).toEqual([]);
      }
    });
  }
}

// a listener's theme that names its loops' rates the older way: stepped to them as they start, held when asked unless
// they show progress, moving again after
test('a theme\'s loops that name a rate (--loop-fps) are stepped to it and held when asked', async ({ page }) => {
  await openScreen(page, '/home', { design: 'petal', theme: 'day' });
  await page.addStyleTag({ content: `@keyframes gate-bloom { to { transform: rotate(40deg); opacity: .4; } }
    .gate-bloom { position: fixed; left: 20px; top: 20px; width: 12px; height: 12px; background: pink; animation: gate-bloom 2.5s ease-in-out infinite alternate; --loop-fps: 12; }
    .gate-bloom--progress { left: 40px; --loop-holds: no; }` });
  await page.evaluate(() => { for (const c of ['gate-bloom', 'gate-bloom gate-bloom--progress']) { const d = document.createElement('i'); d.className = c; document.body.appendChild(d); } });
  await page.waitForTimeout(800);
  const theirs = async () => (await loops(page)).filter((l) => /^gate-bloom/.test(l.name));
  let now = await theirs();
  expect(now.map((l) => Math.round(l.perSecond)), 'the theme\'s loops change 12 times a second, as they name').toEqual([12, 12]);
  await page.evaluate(() => App.still('gate', true));
  now = await theirs();
  expect(now.map((l) => l.paused), 'held when asked, but the one that shows progress').toEqual([true, false]);
  await page.evaluate(() => App.still('gate', false));
  now = await theirs();
  expect(now.map((l) => l.paused), 'moving again after').toEqual([false, false]);
});
