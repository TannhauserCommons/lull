// Shared by the UI gates: open a screen on the fixture hub, slow the CPU, and read what the main thread did from a
// Chrome trace. Gates assert on counts the trace reports (elements restyled, layouts, frames, task lengths), which
// stay put from run to run, rather than on stopwatch times.

import { spawn } from 'node:child_process';
import { createServer } from 'node:net';
import { fileURLToPath } from 'node:url';


export const designs = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
export const themes = ['day', 'night'];
// A fixed "now", as in the visual matrix: relative times and date ranges read the same on every run.
export const NOW = new Date('2026-11-01T12:00:00Z');
// Ids from fixture-hub/library.mjs (seeded, so they never change): the 1,200-song and 60-song playlists, an album.
export const ids = { longPlaylist: 'pl-1', playlist: 'pl-2', album: 'al-ef43dbcb8e61df55', artist: 'ar-f37fb5b576dcae57' };
// The key screens the screens and accessibility gates look at.
export const screens = {
  home: '/home',
  search: '/search?q=velvet',
  playlist: `/playlist/${ids.playlist}`,
  album: `/album/${ids.album}`,
  now: '/now',
  settings: '/settings/playback',
};


// A fixture hub of the gate's own, on a free port, with `env` on top of this process's (library.mjs reads FIXTURE_*): for
// a library of another size, or for changes (likes, a playlist's order) the gates beside it must not see.
export async function ownHub(env = {}) {
  const port = await new Promise((ok, fail) => {
    const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)); }).on('error', fail);
  });
  const server = fileURLToPath(new URL('../fixture-hub/server.mjs', import.meta.url));
  const child = spawn(process.execPath, [server, String(port)], { env: { ...process.env, ...env }, stdio: 'ignore' });
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; ; i++) {
    try { if ((await fetch(url + '/health')).ok) break; } catch { /* not listening yet */ }
    if (i > 100) { child.kill(); throw new Error('the fixture hub did not start on ' + url); }
    await new Promise((ok) => setTimeout(ok, 100));
  }
  return { url, close: () => child.kill() };
}

// Open `route` in `design`/`theme` and wait until the app has booted and the view stopped changing.
export async function openScreen(page, route, { design = 'lull', theme = 'day', clock = true } = {}) {
  if (clock) await page.clock.setFixedTime(NOW);
  await page.addInitScript(([d, t]) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: t })), [design, theme]);
  await page.goto(`/#${route}`);
  await page.waitForFunction(() => window.App && App.server && App.server.checked);
  await settle(page);
}

// Fonts in, and the view's markup unchanged for a moment (screens render in steps as reads settle).
export async function settle(page) {
  await page.evaluate(() => document.fonts.ready);
  let last = '';
  for (let i = 0, stable = 0; i < 80 && stable < 4; i++) {
    await page.waitForTimeout(100);
    const now = await page.evaluate(() => `${document.querySelector('#view')?.innerHTML.length}:${document.images.length}`);
    stable = now === last ? stable + 1 : 0;
    last = now;
  }
}

export async function throttle(page, rate) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Emulation.setCPUThrottlingRate', { rate });
  return cdp;
}

const CATEGORIES = ['devtools.timeline', 'disabled-by-default-devtools.timeline', 'blink.user_timing'];

// Run `fn` under a trace and summarise the page's main thread:
//   tasks      every top-level task's length (ms), so a gate can ask for the longest
//   busyMs     their sum
//   handlersMs time in event handlers (pointer, mouse, key...)
//   frames     main-thread frames committed to the compositor
//   drawn      frames the page's compositor drew (with { drawn: true }: its own trace category), the main thread's
//              and those a compositor-only animation draws with no main-thread work at all
//   layouts    Layout passes; forcedLayouts: those run inside script (a read after a write)
//   restyled   elements restyled by each style pass, largest first
//   marked     the same restyle counts between console.timeStamp('gate-start') and ('gate-end'), when present (not
//              performance.mark: the fixed clock the gates install replaces `performance`)
export async function traced(browser, page, fn, { drawn = false } = {}) {
  await browser.startTracing(page, { categories: drawn ? [...CATEGORIES, 'disabled-by-default-devtools.timeline.frame'] : CATEGORIES });
  let raw;
  try {
    await fn();
  } finally {
    raw = await browser.stopTracing();
  }
  return summarise(JSON.parse(raw.toString()));
}

// Wait until the page's compositor has drawn nothing for `ms` (giving up after `within`): a change just made is on
// screen, so a count that follows sees only what keeps drawing by itself. Two requestAnimationFrames don't get there:
// they wait for the main thread's frames, and the tiles those paint are rastered and drawn after them, a frame or more
// later on a slow machine. A page that never goes quiet is left to the count that follows, which then fails.
export async function quiet(browser, page, { ms = 250, within = 3000 } = {}) {
  for (const end = Date.now() + within; Date.now() < end;) {
    if (!(await traced(browser, page, () => page.waitForTimeout(ms), { drawn: true })).drawn) return;
  }
}

export function summarise(json) {
  const events = json.traceEvents || json;
  const mainThreads = new Set(events.filter((e) => e.name === 'thread_name' && e.args?.name === 'CrRendererMain').map((e) => `${e.pid}:${e.tid}`));
  const busy = {};
  for (const e of events) if (e.name === 'RunTask' && e.ph === 'X' && mainThreads.has(`${e.pid}:${e.tid}`)) busy[`${e.pid}:${e.tid}`] = (busy[`${e.pid}:${e.tid}`] || 0) + e.dur;
  const page = Object.keys(busy).sort((a, b) => busy[b] - busy[a])[0];
  const mine = events.filter((e) => `${e.pid}:${e.tid}` === page && e.ph === 'X');
  const tasks = mine.filter((e) => e.name === 'RunTask').map((e) => e.dur / 1000);
  const scripts = mine.filter((e) => e.name === 'FunctionCall' || e.name === 'EvaluateScript' || e.name === 'TimerFire' || e.name === 'FireAnimationFrame' || e.name === 'EventDispatch');
  const inScript = (e) => scripts.some((s) => e.ts >= s.ts && e.ts + e.dur <= s.ts + s.dur);
  const layouts = mine.filter((e) => e.name === 'Layout');
  const styles = mine.filter((e) => e.name === 'UpdateLayoutTree');
  const marks = events.filter((e) => e.name === 'TimeStamp' && /^gate-(start|end)$/.test(e.args?.data?.message)).map((e) => e.ts).sort((a, b) => a - b);
  const count = (e) => e.args?.elementCount ?? 0;
  return {
    tasks: tasks.sort((a, b) => b - a),
    longestMs: tasks.length ? Math.max(...tasks) : 0,
    busyMs: tasks.reduce((a, b) => a + b, 0),
    handlersMs: mine.filter((e) => e.name === 'EventDispatch').reduce((a, e) => a + e.dur / 1000, 0),
    frames: mine.filter((e) => e.name === 'Commit').length,
    drawn: events.filter((e) => e.name === 'DrawFrame' && page && String(e.pid) === page.split(':')[0]).length,
    layouts: layouts.length,
    forcedLayouts: layouts.filter(inScript).length,
    restyled: styles.map(count).sort((a, b) => b - a),
    marked: marks.length >= 2 ? styles.filter((e) => e.ts >= marks[0] && e.ts <= marks[marks.length - 1]).map(count).sort((a, b) => b - a) : [],
  };
}

// Elements one small DOM change restyles: a class flip, a text change and an insertion, each on its own element at
// the end of <body>. A design rule that reacts to any change on the page (a :has() over the whole page that ends in
// a broad selector, a [class*=] test) turns each of these into a restyle of the whole screen.
// How many times a second a loop that never ends changes, read from its stepped timing (docs/themes.md, Drawing
// cheaply): steps(n) changes n times an iteration, a stepped easing (a linear() staircase: each value held flat, then a
// jump) as many times as it has values; any other timing changes every frame (Infinity). Runs in the page, in any
// engine: pass it as loopRate.toString() and rebuild it there.
export function loopRate(a) {
  const changes = (easing) => {
    const steps = /^steps\((\d+)/.exec(easing || '');
    if (steps) return +steps[1];
    if (!/^linear\(/.test(easing || '')) return Infinity;
    const values = easing.slice(7, -1).split(',').map((s) => s.trim().split(/\s+/)[0]);
    let n = 1; for (let i = 1; i < values.length; i++) if (values[i] !== values[i - 1]) n++;
    return values.length >= 2 * n ? n : Infinity;   // each value held over a stretch: a staircase, not a curve
  };
  const e = a.effect, t = e.getTiming();
  const per = t.easing && t.easing !== 'linear' ? changes(t.easing)
    : e.getKeyframes().slice(0, -1).reduce((sum, k) => sum + changes(k.easing), 0);
  return per / (t.duration / 1000);
}

// The layers the compositor keeps for the page as it stands: how many draw, and their area in windows (the window's
// size each, each layer counted up to one window). Each is a texture kept in GPU memory and drawn again for every frame; a large decoration on a layer of its
// own (a fixed sky) makes what is painted over it need another as large. The tree comes after a compositing update,
// forced by a probe that is then removed and left out.
export async function composited(page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('LayerTree.enable');
  const tree = () => new Promise((res) => {
    const on = (m) => { if (m.layers) { cdp.off('LayerTree.layerTreeDidChange', on); res(m.layers); } };
    cdp.on('LayerTree.layerTreeDidChange', on);
    page.evaluate(() => { const d = document.createElement('div'); d.style.cssText = 'position:fixed;left:0;top:0;width:1px;height:1px;will-change:transform'; document.body.appendChild(d); requestAnimationFrame(() => requestAnimationFrame(() => d.remove())); });
    setTimeout(() => res(null), 5000);
  });
  await tree(); await page.waitForTimeout(200);
  const layers = (await tree()) || [];
  const { w, h } = await page.evaluate(() => ({ w: innerWidth, h: innerHeight }));
  const drawing = layers.filter((l) => l.drawsContent && !(l.width === 1 && l.height === 1));
  await cdp.detach();
  // a list's scrolling layer is as long as the list, but only what is near the window is drawn: each counts a window at most
  return { layers: drawing.length, windows: +(drawing.reduce((s, l) => s + Math.min(l.width, w) * Math.min(l.height, h), 0) / (w * h)).toFixed(2) };
}

export async function restyleFanOut(browser, page) {
  const out = {};
  for (const kind of ['class', 'text', 'insert']) {
    const s = await traced(browser, page, () =>
      page.evaluate((k) => {
        const el = document.body.appendChild(document.createElement('div'));
        el.textContent = 'probe';
        document.body.offsetWidth;
        console.timeStamp('gate-start');
        for (let i = 0; i < 3; i++) {
          if (k === 'class') el.classList.toggle('is-on');
          else if (k === 'text') el.textContent = 'probe ' + i;
          else {   // as a menu or toast arrives and goes
            const x = document.body.appendChild(document.createElement('div'));
            document.body.offsetWidth;
            x.remove();
          }
          document.body.offsetWidth;
        }
        console.timeStamp('gate-end');
        el.remove();
      }, kind),
    );
    out[kind] = s.marked[0] ?? 0;
  }
  return out;
}
