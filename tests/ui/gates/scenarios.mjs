// What a listener does, scripted: each scenario drives the page and returns the trace summary of the part that
// matters (lib.mjs `summarise`), plus its own facts. perf.spec.mjs asserts budgets on these; the same functions
// print before/after numbers when run by hand (docs/context/quality.md#ui-gates).

import { fileURLToPath } from 'node:url';
import { ids, openScreen, settle, throttle, traced } from './lib.mjs';

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
  return traced(browser, page, async () => {
    for (let i = 0; i < 24; i++) {
      await page.mouse.wheel(0, 400);
      await page.waitForTimeout(50);
    }
    await page.waitForTimeout(300);
  });
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
  return s;
}

// Nothing playing, nothing moving: what the page costs at rest.
export async function idle(browser, page, opts, route = '/home') {
  await openScreen(page, route, opts);
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
  await throttle(page, opts.rate);
  const s = await traced(browser, page, () => page.waitForTimeout(2000));
  s.perSecondMs = s.busyMs / 2;
  // what was moving, to name the cause when frames show up at rest
  s.animations = await page.evaluate(() => [...new Set(document.getAnimations().filter((a) => a.playState === 'running').map((a) => {
    const t = a.effect?.target, kf = a.effect?.getKeyframes?.() || [];
    const props = [...new Set(kf.flatMap((k) => Object.keys(k).filter((p) => !['offset', 'easing', 'composite', 'computedOffset'].includes(p))))];
    const timeline = a.timeline && a.timeline.constructor.name !== 'DocumentTimeline' ? ' (' + a.timeline.constructor.name + ')' : '';
    return (a.animationName || a.transitionProperty || 'animation') + ' on ' + (t ? t.tagName.toLowerCase() + '.' + [...t.classList].slice(0, 2).join('.') : '?') + (a.effect?.pseudoElement || '') + ': ' + props.join(',') + timeline;
  }))]);
  return s;
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
