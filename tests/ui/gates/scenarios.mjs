// What a listener does, scripted: each scenario drives the page and returns the trace summary of the part that
// matters (lib.mjs `summarise`), plus its own facts. perf.spec.mjs asserts budgets on these; the same functions
// print before/after numbers when run by hand (docs/context/quality.md#ui-gates).

import { fileURLToPath } from 'node:url';
import { composited, ids, loopRate, openScreen, ownHub, settle, throttle, traced } from './lib.mjs';

const playShown = (page) =>
  page.evaluate(() => App.player.playList([...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id')), 0));

// rows fully on screen, clear of the header and the player bar (the list scrolled to start a few rows in)
const rowsOf = async (page) => {
  await page.locator('#view .tlist > .trow').nth(10).evaluate((r) => r.scrollIntoView({ block: 'center' }));
  await page.waitForTimeout(200);
  return page.$$eval('#view .tlist > .trow', (els) =>
    els
      .map((e) => e.getBoundingClientRect())
      .filter((r) => r.top > 80 && r.bottom < innerHeight - 120)
      .map((r) => ({ x: r.left + r.width * 0.45, y: r.top + r.height / 2 })),
  );
};

// Startup: from navigation until Home has rendered its data.
export async function startup(browser, page, opts) {
  await throttle(page, opts.rate);
  const s = await traced(browser, page, () => openScreen(page, '/home', opts));
  s.domNodes = await page.evaluate(() => document.getElementsByTagName('*').length);
  return s;
}

// The pointer sweeps down and up a long playlist, one move a frame. `lands` is how many frames the highlight takes to
// reach its full colour on the row the pointer enters.
export async function hover(browser, page, opts) {
  await openScreen(page, `/playlist/${ids.longPlaylist}`, opts);
  const rows = await rowsOf(page);
  await throttle(page, opts.rate);
  await page.mouse.move(rows[0].x, rows[0].y - 30);
  const s = await traced(browser, page, async () => {
    for (let pass = 0; pass < 2; pass++) {
      for (const r of pass ? [...rows].reverse() : rows) {
        await page.mouse.move(r.x, r.y - 8);
        await page.mouse.move(r.x, r.y + 8, { steps: 2 });
      }
    }
  });
  // frames from the pointer entering a row until its background stops changing (0: the first frame has it)
  await page.mouse.move(rows[3].x, rows[3].y);
  s.landsInFrames = await page.evaluate(async (r) => {
    const row = document.elementFromPoint(r.x, r.y).closest('.trow');
    const frame = () => new Promise((ok) => requestAnimationFrame(() => ok()));
    const seen = [];
    for (let i = 0; i < 20; i++) { seen.push(getComputedStyle(row).backgroundColor); await frame(); }
    return seen.indexOf(seen[seen.length - 1]);
  }, rows[3]);
  return s;
}

// Scroll a long playlist a screen at a time with the wheel.
export async function scroll(browser, page, opts) {
  await openScreen(page, `/playlist/${ids.longPlaylist}`, opts);
  const box = await page.locator('#view').boundingBox();
  await page.mouse.move(box.x + box.width / 2, box.y + 300);
  await throttle(page, opts.rate);
  const s = await traced(browser, page, async () => {
    for (let i = 0; i < 24; i++) {
      await page.mouse.wheel(0, 400);
      await page.waitForTimeout(50);
    }
    await page.waitForTimeout(300);
  });
  s.domNodes = await page.evaluate(() => document.getElementsByTagName('*').length);
  return s;
}

// A playlist of `songs` scrolled top to bottom in a fresh page, on a fixture hub of its own (FIXTURE_LONG_PLAYLIST; the
// library the same size at any length, FIXTURE_ALBUMS, so only the list differs), then what the renderer holds after a
// garbage collection: its whole private footprint (DOM, layout, styles, decoded pictures, the JS heap; also once opened,
// before the scroll), DOM nodes, elements and the empty rows among them (ui.row), and pictures.
export async function listCost(browser, opts, songs) {
  const hub = await ownHub({ FIXTURE_ALBUMS: '480', FIXTURE_LONG_PLAYLIST: String(songs) });
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 }, serviceWorkers: 'block', deviceScaleFactor: 1 });
  try {
    const page = await context.newPage();
    await openScreen(page, `/playlist/${ids.longPlaylist}`, opts);
    const cdp = await context.newCDPSession(page);
    const collect = async () => { for (let i = 0; i < 2; i++) { await cdp.send('HeapProfiler.collectGarbage'); await page.waitForTimeout(500); } };
    await collect();
    const openedMB = await rendererMB(browser);
    await page.evaluate(async () => {
      let s = document.querySelector('#view li[data-fill]');
      while (s && !(s.scrollHeight > s.clientHeight + 10 && /auto|scroll/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
      s = s || document.scrollingElement;
      for (let y = 0; y < s.scrollHeight; y += 800) { s.scrollTop = y; await new Promise((ok) => requestAnimationFrame(() => setTimeout(ok, 30))); }
    });
    await settle(page);
    await collect();
    const { nodes } = await cdp.send('Memory.getDOMCounters');
    const facts = await page.evaluate(() => ({
      rows: document.querySelectorAll('#view .tlist > li[data-track-id]').length,
      elements: document.getElementsByTagName('*').length,
      empty: document.querySelectorAll('li[data-fill]:empty').length,
      images: document.images.length,
    }));
    return { songs, nodes, ...facts, openedMB, rendererMB: await rendererMB(browser) };
  } finally {
    await context.close();
    hub.close();
  }
}
// the largest renderer's private footprint (MB) in a memory dump: the page measured, the only one open
async function rendererMB(browser) {
  const cdp = await browser.newBrowserCDPSession(), events = [];
  cdp.on('Tracing.dataCollected', (e) => events.push(...e.value));
  const done = new Promise((ok) => cdp.once('Tracing.tracingComplete', ok));
  await cdp.send('Tracing.start', { transferMode: 'ReportEvents', traceConfig: { includedCategories: ['disabled-by-default-memory-infra'], excludedCategories: ['*'], memoryDumpConfig: { triggers: [] } } });
  await cdp.send('Tracing.requestMemoryDump', { deterministic: true, levelOfDetail: 'detailed' });
  await cdp.send('Tracing.end');
  await done;
  await cdp.detach();
  const names = {};
  for (const e of events) if (e.ph === 'M' && e.name === 'process_name') names[e.pid] = e.args.name;
  const mb = events.filter((e) => e.ph === 'v' && /renderer/i.test(names[e.pid] || '') && e.args.dumps.process_totals?.private_footprint_bytes)
    .map((e) => parseInt(e.args.dumps.process_totals.private_footprint_bytes, 16) / 1048576);
  return Math.max(...mb);
}

// Home, the long playlist, an album, Home again; returns the longest task of each move.
export async function navigate(browser, page, opts) {
  await openScreen(page, '/home', opts);
  await throttle(page, opts.rate);
  const out = { steps: {} };
  for (const route of [`/playlist/${ids.longPlaylist}`, `/album/${ids.album}`, '/library/songs', '/home']) {
    const s = await traced(browser, page, async () => {
      await page.evaluate((r) => { location.hash = r; }, route);
      await settle(page);
    });
    out.steps[route] = s;
  }
  out.longestMs = Math.max(...Object.values(out.steps).map((s) => s.longestMs));
  return out;
}

// Type a query in the header search, a key every 120 ms.
export async function typeSearch(browser, page, opts) {
  await openScreen(page, '/search', opts);
  const input = page.locator('.header__search input, #view input[type="search"]').first();
  await input.click();
  await throttle(page, opts.rate);
  const s = await traced(browser, page, async () => {
    await input.pressSequentially('velvet harbour', { delay: 120 });
    await page.waitForTimeout(600);
  });
  return s;
}

// Open and close a song's menu on the long playlist (right-click opens the same menu as a row's "…").
export async function openMenu(browser, page, opts) {
  await openScreen(page, `/playlist/${ids.longPlaylist}`, opts);
  const row = page.locator('#view .tlist > .trow').nth(6);
  await row.scrollIntoViewIfNeeded();
  await throttle(page, opts.rate);
  return traced(browser, page, async () => {
    for (let i = 0; i < 3; i++) {
      await row.click({ button: 'right' });
      await page.locator('.menu').waitFor();
      await page.keyboard.press('Escape');
      await page.locator('.menu').waitFor({ state: 'detached' });
    }
  });
}

// Start a playlist, open the queue in the side panel, drag a song down five places. (A 1,200-song queue re-renders
// every row on a move, about 100 ms unthrottled: windowing long lists is the fix, docs/context/quality.md#ui-gates.)
export async function queueDrag(browser, page, opts, playlist = ids.playlist) {
  await openScreen(page, `/playlist/${playlist}`, opts);
  await playShown(page);
  await page.waitForFunction(() => App.player.state.queue.length > 20);
  await page.evaluate(() => App.openPanel('queue'));   // the side panel's queue, beside the long playlist
  await settle(page);
  const rows = page.locator('.ctx .now-q__row[data-sortable]');
  await rows.nth(1).waitFor();
  const a = await rows.nth(1).boundingBox(), b = await rows.nth(6).boundingBox();
  await throttle(page, opts.rate);
  const s = await traced(browser, page, async () => {
    await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2);
    await page.mouse.down();
    for (let k = 1; k <= 30; k++) await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2 + ((b.y - a.y) * k) / 30);
    await page.mouse.up();
    await page.waitForTimeout(300);
  });
  s.queue = await page.evaluate(() => App.player.state.queue.length);
  return s;
}

// Drag the side panel's resize handle back and forth (docked at 1440 px).
export async function resizePanel(browser, page, opts) {
  await openScreen(page, `/playlist/${ids.playlist}`, opts);
  const handle = page.locator('[data-resize="panel"]');
  if (!(await handle.isVisible())) return null;
  const h = await handle.boundingBox();
  await throttle(page, opts.rate);
  return traced(browser, page, async () => {
    await page.mouse.move(h.x + h.width / 2, h.y + 200);
    await page.mouse.down();
    for (let k = 0; k <= 40; k++) await page.mouse.move(h.x + h.width / 2 - 120 * Math.sin((k / 40) * Math.PI), h.y + 200);
    await page.mouse.up();
    await page.waitForTimeout(200);
  });
}

// Two seconds with a song playing, on `route`: the per-second cost of the clock, seek bars and designs' loops.
export async function playing(browser, page, opts, route = '/home') {
  await openScreen(page, `/album/${ids.album}`, opts);
  await playShown(page);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.5);
  await page.evaluate((r) => App.go(r), route);
  await settle(page);
  await page.waitForTimeout(1500);   // past the screen's entrance transitions: the steady cost of playing
  await throttle(page, opts.rate);
  const s = await traced(browser, page, () => page.waitForTimeout(2000));
  s.perSecondMs = s.busyMs / 2;
  s.framesPerSecond = s.frames / 2;
  s.loops = await loops(page);
  return s;
}

// Nothing playing, nothing moving: what the page costs at rest.
// each animation the trace says couldn't run on the compositor: its name, its element and Chromium's reasons (a bitmask:
// CompositorAnimations::FailureReason) and properties
function offTheCompositor(events) {
  // in time order: an id is used again once its animation ends, so a reason belongs to the one that began last with it
  const anim = events.filter((e) => e.name === 'Animation').sort((x, y) => x.ts - y.ts), named = {}, out = new Set();
  for (const e of anim) {
    const id = e.id2?.local ?? e.id, d = e.args?.data;
    if (e.ph === 'b' && d) named[id] = `${d.displayName || 'animation'} on ${d.nodeName || '?'}`;
    else if (d?.compositeFailed) out.add(`${named[id] || 'an animation'} (reasons ${d.compositeFailed}${d.unsupportedProperties ? ', ' + d.unsupportedProperties.join(' ') : ''})`);
  }
  return [...out];
}

// What made a traced window's main-thread frames, for the message of a count that fails (a frame at rest is rare and
// comes and goes, so the failure itself has to name it): the animations that began, ended or left the compositor in
// the window, what was invalidated and why, the script that ran, and frames another renderer committed
function whatRan(events, thread) {
  const pid = (thread || '').split(':')[0], on = (e) => `${e.pid}:${e.tid}` === thread;
  const top = (counts, n) => Object.entries(counts).sort((x, y) => y[1] - x[1]).slice(0, n).map(([k, c]) => (c > 1 ? `${k} x${c}` : k));
  const tally = (list, key) => list.reduce((c, e) => { const k = key(e); if (k) c[k] = (c[k] || 0) + 1; return c; }, {});
  const anim = events.filter((e) => e.name === 'Animation' && String(e.pid) === pid);
  const began = tally(anim.filter((e) => e.ph === 'b'), (e) => `${e.args?.data?.displayName || 'animation'} on ${e.args?.data?.nodeName || '?'}`);
  const ended = anim.filter((e) => e.ph === 'e').length;
  const invalid = tally(events.filter((e) => String(e.pid) === pid && /InvalidationTracking$/.test(e.name)), (e) => {
    const d = e.args?.data || {};
    return `${e.name.replace(/InvalidationTracking$/, '').replace(/^StyleRecalc|^StyleInvalidator/, 'Style')} ${d.nodeName || '?'}${d.reason ? ' (' + d.reason + ')' : ''}`;
  });
  const script = tally(events.filter((e) => on(e) && e.ph === 'X'), (e) => {
    const d = e.args?.data || {};
    if (e.name === 'EventDispatch') return 'event ' + d.type;
    if (e.name === 'TimerFire') return 'timer';
    if (e.name === 'FireAnimationFrame') return 'requestAnimationFrame';
    if (e.name === 'FunctionCall') return `${d.functionName || 'function'} (${(d.url || '').split('/').pop()}:${d.lineNumber ?? '?'})`;
    return null;
  });
  const elsewhere = events.filter((e) => e.name === 'Commit' && e.ph === 'X' && !on(e)).length;
  const parts = [];
  if (Object.keys(began).length) parts.push('animations begun: ' + top(began, 6).join(', '));
  if (ended) parts.push(ended + ' ended');
  const off = offTheCompositor(events.filter((e) => String(e.pid) === pid));
  if (off.length) parts.push('off the compositor: ' + off.join(', '));
  if (Object.keys(invalid).length) parts.push('invalidated: ' + top(invalid, 6).join(', '));
  if (Object.keys(script).length) parts.push('ran: ' + top(script, 5).join(', '));
  if (elsewhere) parts.push(elsewhere + ' frames committed by another renderer');
  return parts.join('; ') || 'no animation, invalidation or script in the trace';
}

export async function idle(browser, page, opts, route = '/home') {
  // as a listener leaves it: the desktop app's bridge in (its windows are fed while they're open), a song loaded and
  // paused, the pointer off the page
  await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
  /* from the start, the animations Chromium couldn't hand to the compositor and why (blink.animations: an animation
     says so as it starts, before the measured window): a loop that runs on the main thread draws main-thread frames at
     rest, and this names it when they show up */
  await browser.startTracing(page, { categories: ['blink.animations'] });
  await openScreen(page, `/album/${ids.album}`, opts);
  await playShown(page);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.3);
  await page.evaluate(() => { App.player.pause(); window.__bridge.fire('mini', { action: 'hello' }); });
  await page.evaluate((r) => { location.hash = r; }, route);
  await settle(page);
  await page.mouse.move(1, 1);
  await page.waitForTimeout(1500);   // past entrances and cover fade-ins
  // and, on a slow runner, until they have happened: every cover on screen in, every animation on screen that ends
  // ended (a design's loops and scroll-driven effects never end, and are what this gate is about). Polled on a timer:
  // polling on frames would draw the frames this counts
  await page.waitForFunction(() => {
    // drawn (not hidden, nor in a row content-visibility skips), and inside the window and every box that clips it
    const shown = (el) => {
      if (!el || !el.checkVisibility({ contentVisibilityAuto: true, visibilityProperty: true })) return false;
      const r = el.getBoundingClientRect();
      let [t, b, l, x] = [Math.max(r.top, 0), Math.min(r.bottom, innerHeight), Math.max(r.left, 0), Math.min(r.right, innerWidth)];
      for (let e = el.parentElement; e && b > t && x > l; e = e.parentElement) {
        if (getComputedStyle(e).overflow === 'visible') continue;
        const c = e.getBoundingClientRect();
        [t, b, l, x] = [Math.max(t, c.top), Math.min(b, c.bottom), Math.max(l, c.left), Math.min(x, c.right)];
      }
      return b > t && x > l;
    };
    return [...document.images].every((i) => i.complete || !shown(i))
      && document.getAnimations().every((a) => a.playState !== 'running' || a.effect?.getTiming().iterations === Infinity
        || a.timeline?.constructor.name !== 'DocumentTimeline' || !shown(a.effect?.target));
  }, null, { polling: 100, timeout: 5000 }).catch(() => {});
  const offCompositor = offTheCompositor(JSON.parse((await browser.stopTracing()).toString()).traceEvents || []);
  await throttle(page, opts.rate);
  const s = await traced(browser, page, () => page.waitForTimeout(2000), { drawn: true, why: true });
  s.why = whatRan(s.events, s.thread);
  delete s.events;
  s.perSecondMs = s.busyMs / 2;
  s.offCompositor = offCompositor;
  // what was moving, to name the cause when frames show up at rest
  s.animations = await page.evaluate(() => [...new Set(document.getAnimations().filter((a) => a.playState === 'running').map((a) => {
    const t = a.effect?.target, kf = a.effect?.getKeyframes?.() || [];
    const props = [...new Set(kf.flatMap((k) => Object.keys(k).filter((p) => !['offset', 'easing', 'composite', 'computedOffset'].includes(p))))];
    const timeline = a.timeline && a.timeline.constructor.name !== 'DocumentTimeline' ? ' (' + a.timeline.constructor.name + ')' : '';
    return (a.animationName || a.transitionProperty || 'animation') + ' on ' + (t ? t.tagName.toLowerCase() + '.' + [...t.classList].slice(0, 2).join('.') : '?') + (a.effect?.pseudoElement || '') + ': ' + props.join(',') + timeline;
  }))]);
  s.loops = await loops(page);
  s.composited = await composited(page);
  return s;
}

// The loops on screen that draw more than they need to (docs/themes.md, Drawing cheaply): each one that never ends must
// change at most 30 times a second, stepped where it is declared (steps(n), or a stepped easing), move only by transform
// and opacity (the compositor's, no repaint), and stay clear of live glass (a backdrop-filter recomputes its blur for
// every frame that changes anything under or inside it). A scroll-driven effect (an animation-timeline of scroll() or
// view(), a fade at a list's foot) changes only as the page scrolls, so its rate and glass don't count at rest; but one
// that animates anything besides transform and opacity runs on the main thread, which services it every frame, scrolled
// or not: a main-thread frame at every refresh, at rest. That is read from its keyframes, the same on every runner,
// where the frames it costs depend on the runner. Named, with the reason, for the failure.
export function loops(page) {
  return page.evaluate((src) => {
    const rate = (0, eval)('(' + src + ')');
    const glass = [...document.querySelectorAll('body *')].filter((e) => {
      const s = getComputedStyle(e), f = s.backdropFilter || s.webkitBackdropFilter;
      return f && f !== 'none' && e.checkVisibility();
    });
    const hit = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom && a.width && a.height;
    // what each @keyframes animates, read from the sheets: getKeyframes() leaves custom properties out (Starlight's
    // scroll fade animated --sl-fade, which the compositor can't run, and read as animating nothing)
    const animates = new Map();
    const walk = (rules) => {
      for (const rule of rules) {
        if (rule instanceof CSSKeyframesRule) {
          const set = animates.get(rule.name) || new Set();
          for (const k of rule.cssRules) for (let i = 0; i < k.style.length; i++) if (k.style[i] !== 'animation-timing-function') set.add(k.style[i]);
          animates.set(rule.name, set);
        } else if (rule.cssRules) walk(rule.cssRules);
      }
    };
    for (const sheet of document.styleSheets) { try { walk(sheet.cssRules); } catch { /* another origin's sheet */ } }
    const kebab = (p) => p.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
    const out = [];
    for (const a of document.getAnimations()) {
      const e = a.effect, t = e?.target;
      const scrolled = a.timeline && a.timeline.constructor.name !== 'DocumentTimeline';
      if (!t || a.playState !== 'running' || (!scrolled && e.getTiming().iterations !== Infinity)) continue;
      if (!t.checkVisibility({ contentVisibilityAuto: true, visibilityProperty: true })) continue;
      // what of it shows: its box clipped by every box that clips it (a ticker's long track shows only in its strip)
      const r = t.getBoundingClientRect().toJSON();
      for (let c = t.parentElement; c; c = c.parentElement) {
        if (getComputedStyle(c).overflow === 'visible') continue;
        const k = c.getBoundingClientRect();
        r.left = Math.max(r.left, k.left); r.right = Math.min(r.right, k.right); r.top = Math.max(r.top, k.top); r.bottom = Math.min(r.bottom, k.bottom);
      }
      r.width = r.right - r.left; r.height = r.bottom - r.top;
      if (r.width <= 0 || r.height <= 0 || r.bottom <= 0 || r.top >= innerHeight || r.right <= 0 || r.left >= innerWidth) continue;
      const name = (a.animationName || 'animation') + ' on ' + t.tagName.toLowerCase() + '.' + [...t.classList].slice(0, 2).join('.') + (e.pseudoElement || '');
      const props = [...new Set([
        ...e.getKeyframes().flatMap((k) => Object.keys(k).filter((p) => !['offset', 'easing', 'composite', 'computedOffset'].includes(p)).map(kebab)),
        ...(animates.get(a.animationName) || []),
      ])];
      const slow = props.filter((p) => !['transform', 'opacity', 'translate', 'scale', 'rotate'].includes(p));
      if (scrolled) {
        if (slow.length) out.push(name + ' (' + a.timeline.constructor.name + '): runs on the main thread, every frame (' + slow.join(', ') + ')');
        continue;
      }
      const perSecond = rate(a);
      if (perSecond > 30.5) out.push(name + ': ' + (perSecond === Infinity ? 'changes every frame (no stepped timing)' : 'changes ' + Math.round(perSecond) + ' times a second'));
      if (slow.length) out.push(name + ': repaints (' + slow.join(', ') + ')');
      const under = glass.find((g) => hit(r, g.getBoundingClientRect()));
      if (under) out.push(name + ': under or in live glass (' + under.tagName.toLowerCase() + '.' + [...under.classList].slice(0, 2).join('.') + ')');
    }
    return out;
  }, loopRate.toString());
}

// A long listening session, compressed: the desktop app's bridge stubbed in (desktop-bridge.js), a queue skipped
// through song by song while the screens change, and what stays behind measured after a forced collection: heap,
// nodes, listeners, and what crossed the bridge per song.
export async function listening(browser, page, opts) {
  // every seventh album's cover is one the hub is still fetching (404 until art.ready), as on a real hub: a cover
  // waiting for its picture must not keep its screen alive once it leaves the page
  const { albumIds } = await import('../fixture-hub/library.mjs');
  const pending = albumIds.filter((_, i) => i % 7 === 3);
  // in its own scope on the hub (server.mjs, X-Fixture-Client), so the gates beside it never see these covers pending
  const headers = { 'x-fixture-client': 'listening:' + Math.random().toString(36).slice(2) };
  await page.context().setExtraHTTPHeaders(headers);
  const fixture = (p, ids) => page.request.post(p, { data: { ids }, headers });
  await fixture('/__fixture/art-pending', pending);
  await page.addInitScript({ path: fileURLToPath(new URL('./desktop-bridge.js', import.meta.url)) });
  await openScreen(page, `/playlist/${ids.playlist}`, opts);   // a song from another album each time: a new cover each time
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('Performance.enable');
  // measured on the same screen each time, so a screen's own size doesn't count as growth
  const heap = async () => {
    await page.evaluate(() => { location.hash = '/home'; });
    await settle(page);
    await cdp.send('HeapProfiler.collectGarbage');
    const m = Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
    const attached = await page.evaluate(() => document.getElementsByTagName('*').length);
    return { heapMB: m.JSHeapUsedSize / 1e6, nodes: m.Nodes, attached, listeners: m.JSEventListeners };
  };
  await page.evaluate(() => {
    const ids = [...document.querySelectorAll('#view .trow[data-track-id]')].map((r) => r.getAttribute('data-track-id'));
    App.player.playList(ids, 0);
    for (let i = 0; i < 3 && App.player.state.repeat !== 'all'; i++) App.player.cycleRepeat();
    window.__bridge.fire('mini', { action: 'hello' });   // a mini player is open: main feeds it
  });
  const screens = ['/now', '/home', `/playlist/${ids.playlist}`, '/search?q=velvet', '/library/songs', `/album/${ids.album}`];
  const play = async (n, from) => {
    for (let k = 0; k < n; k++) {
      await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.3);
      await page.waitForTimeout(400);   // a few clock ticks per song
      if ((from + k) % 4 === 0) { await page.evaluate((r) => { location.hash = r; }, screens[((from + k) / 4) % screens.length]); await settle(page); }
      await page.evaluate(() => App.player.next());
    }
  };
  // a whole lap of the queue (repeat all) between the two samples, so the queue panel shows the same songs both times
  const songs = await page.evaluate(() => App.player.state.queue.length);
  await play(12, 0);   // warm: every screen built once, covers and code paths seen
  const first = await heap();
  const bridge0 = await page.evaluate(() => JSON.parse(JSON.stringify(window.__bridge)));
  await play(songs, 12);
  const last = await heap();
  const bridge = await page.evaluate(() => window.__bridge);
  await fixture('/__fixture/art-ready', pending);
  const sent = (name) => (bridge.bytes[name] || 0) - (bridge0.bytes[name] || 0);
  return {
    first, last, songs,
    heapGrowthMB: last.heapMB - first.heapMB, nodeGrowth: last.nodes - first.nodes, listenerGrowth: last.listeners - first.listeners,
    bridgeKBPerSong: Object.keys(bridge.bytes).reduce((a, k) => a + sent(k), 0) / 1000 / songs,
    largestPlayerSnapshot: bridge.largest.player || 0,
    coversSent: (bridge.calls['player-art'] || 0) - (bridge0.calls['player-art'] || 0),
  };
}
