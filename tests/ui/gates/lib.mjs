// Shared by the UI gates: open a screen on the fixture hub, slow the CPU, and read what the main thread did from a
// Chrome trace. Gates assert on counts the trace reports (elements restyled, layouts, frames, task lengths), which
// stay put from run to run, rather than on stopwatch times.

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
