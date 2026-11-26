// The files a first load of a route fetches, as a real browser lists them: `listRoutes(chromium, tree, dist, routes)`.
//
// The tree's fixture hub (tests/ui/fixture-hub: no Soulseek, no outside service, the same answers every run) serves the
// tree's built web files, and Chromium opens each route as a first visit would: a fresh context for every route, Lighthouse's
// phone (412x823 at 1.75x), light mode, the default design, no stored settings, no service worker. Every same-origin request
// whose path is a file of the built tree is listed (documents, scripts, styles, fonts, images and files fetched from script
// alike); the hub's own answers (/api, /health) are not files of the tree.
//
// A start is what has loaded by a fixed horizon of the page's own time, not by the moment the machine happened to go quiet:
// Playwright's clock is installed before the page loads and paused, and time is advanced in steps up to the horizon, the
// network allowed to finish after each, so every timer, animation frame and idle callback due inside the horizon has run, in
// the same order, on every machine however busy. The horizon is a second short of the first thing lull defers on purpose:
// Classic mode, which app.js loads 15 s after the page is ready (classicDelay reads that from app.js, so moving it moves the
// horizon, and a delay too short to leave a start fails the listing). Outside the start: Classic, the UI sounds after a first
// tap or key (App.needSounds), anything a click opens, and what a browser fetches on its own after load (the page's icons
// and manifest) or that a service worker (blocked here) installs.
//
// Waiting for the network waits on events, never a sleep, and only up to a real-time bound that fails naming what is still in
// flight. A request that is meant to stay open is recognised by what it is: an EventSource or a WebSocket by its type, and a
// response that says it is a `text/event-stream`. The page's own handlers run after the browser has reported a response
// finished (a script's `load` is a task of its own, and what it fetches starts then), so a round of waiting ends with a
// message through a MessageChannel, a task queued behind those handlers, and goes round again while a request began.
import { spawn } from 'node:child_process';
import { existsSync, readFileSync, statSync } from 'node:fs';
import { createServer } from 'node:net';
import { join, posix } from 'node:path';

// Lighthouse's mobile profile as docs/context/quality.md describes it, in light mode.
export const PHONE = {
  viewport: { width: 412, height: 823 }, deviceScaleFactor: 1.75, isMobile: true, hasTouch: true,
  serviceWorkers: 'block', locale: 'en-GB', timezoneId: 'UTC', colorScheme: 'light',
};
// The page's clock starts here (the gates' fixed "now"), and time is advanced in steps of this much virtual time.
export const CLOCK_START = new Date('2026-11-01T12:00:00Z');
export const STEP_MS = 250;
// A start shorter than this is not a start: Classic's delay was moved too close, and what a start is needs deciding again.
export const MIN_HORIZON_MS = 10_000;
// How long, in real time, the network may take to finish after a step before the listing fails naming what is in flight.
export const NETWORK_LIMIT_MS = 30_000;

// Ports Chromium and Node refuse to connect to (the WHATWG fetch standard's list of bad ports): a hub there can't be reached.
const BLOCKED_PORTS = new Set([
  1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95, 101, 102, 103, 104, 109, 110, 111,
  113, 115, 117, 119, 123, 135, 137, 139, 143, 161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548,
  554, 556, 563, 587, 601, 636, 989, 990, 993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 4190, 5060, 5061, 6000, 6566,
  6665, 6666, 6667, 6668, 6669, 6679, 6697, 10080,
]);

// How long after the page is ready app.js defers Classic mode, in ms, read from its source: the `App.on('ready', ...)`
// block that calls App.needClassic and its setTimeout's delay.
export function classicDelay(appJs) {
  const blocks = [...appJs.matchAll(/App\.on\('ready', function \(\) \{([\s\S]*?)\n  \}\);/g)].map((m) => m[1]);
  const block = blocks.find((b) => b.includes('App.needClassic()'));
  const delay = block && /\n\s*\},\s*(\d+)\);\s*$/.exec(block)?.[1];
  if (!delay) throw new Error("app.js no longer loads Classic mode from `App.on('ready', ... setTimeout(..., <ms>))`: update classicDelay in scripts/budgets/list.mjs, the start's horizon comes from it");
  return Number(delay);
}

// The horizon of a start, in ms of the page's own time: a second short of Classic's load.
export function horizonOf(appJs) {
  const horizon = classicDelay(appJs) - 1000;
  if (horizon < MIN_HORIZON_MS) throw new Error(`Classic mode now loads ${horizon + 1000} ms after the page is ready, which leaves a start of ${horizon} ms (at least ${MIN_HORIZON_MS} ms is expected): decide what a start is, in scripts/budgets/list.mjs`);
  return horizon;
}

// The path in the built tree that `url` is a file of, or null: another origin, the hub's own answers, anything not a file.
export function builtFile(url, origin, dist) {
  const u = new URL(url);
  if (u.origin !== origin) return null;
  let path;
  try { path = decodeURIComponent(u.pathname); } catch { return null; }
  const rel = posix.normalize(path.replace(/^\/+/, '') || 'index.html');
  if (rel.startsWith('..')) return null;
  const file = join(dist, rel);
  return existsSync(file) && statSync(file).isFile() ? rel : null;
}

// Whether the page opened the screen `route` names. A key that matches no screen draws the app's "Nothing lives here"
// (no route in App.current), and one that redirects ends elsewhere: either would list the shell and pass for a route.
export function assertScreen(route, state) {
  if (state.routed && state.path === `/${route}`) return;
  const shown = state.routed ? `redirected to ${state.path}` : 'matched no screen';
  throw new Error(`the route "${route}" is not a screen of this app (the page ${shown}): fix its key in scripts/budgets/shipped-bytes.json`);
}

async function freePort() {
  for (;;) {
    const port = await new Promise((ok, fail) => {
      const s = createServer().listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => ok(port)); }).on('error', fail);
    });
    if (!BLOCKED_PORTS.has(port)) return port;
  }
}

// Starts the fixture hub on `port` and says when it is ours and listening ('up'), or that the port was taken between
// being chosen and being bound ('in-use'); anything else that stops it is an error with what it printed.
export function listening(child, port) {
  return new Promise((resolve, reject) => {
    let output = '';
    const timer = setTimeout(() => reject(new Error(`the fixture hub did not say it was listening on ${port} in 30 s: ${output.trim()}`)), 30_000);
    const take = (chunk) => {
      output += chunk;
      if (output.includes(`fixture hub on http://127.0.0.1:${port}`)) { clearTimeout(timer); resolve('up'); }
    };
    child.stdout.on('data', take);
    child.stderr.on('data', take);
    child.on('exit', () => {
      clearTimeout(timer);
      if (output.includes('EADDRINUSE')) resolve('in-use');
      else reject(new Error(`the fixture hub stopped before it was listening on ${port}: ${output.trim()}`));
    });
  });
}

// The tree's own fixture hub, serving `dist`, on a port that is free, not a blocked one, and ours: it says when it listens,
// so another program's answer on the same port is never taken for it. `close()` stops it.
export async function startHub(tree, dist) {
  const server = join(tree, 'tests/ui/fixture-hub/server.mjs');
  for (let attempt = 1; attempt <= 5; attempt++) {
    const port = await freePort();
    const child = spawn(process.execPath, [server, String(port)], { env: { ...process.env, LULL_WEB: dist }, stdio: ['ignore', 'pipe', 'pipe'] });
    let state;
    try { state = await listening(child, port); } catch (err) { child.kill(); throw err; }
    if (state === 'up') return { url: `http://127.0.0.1:${port}`, close: () => child.kill() };
    console.error(`budgets: port ${port} was taken before the fixture hub bound it (EADDRINUSE); trying another`);
  }
  throw new Error('the fixture hub found no free port in 5 tries');
}

// Resolves once nothing tracked is in flight, on the request events themselves, or fails after a real-time bound naming
// what is.
const drained = (page, pending) => new Promise((resolve, reject) => {
  const off = () => { for (const e of ['requestfinished', 'requestfailed', 'response']) page.off(e, check); clearTimeout(timer); };
  const check = () => { if (!pending.size) { off(); resolve(); } };
  const timer = setTimeout(() => {
    off();
    const inFlight = [...pending].map((r) => `${r.method()} ${r.url()} (${r.resourceType()})`).join(', ');
    reject(new Error(`still in flight ${NETWORK_LIMIT_MS / 1000} s after the page's last step: ${inFlight}`));
  }, NETWORK_LIMIT_MS);
  for (const e of ['requestfinished', 'requestfailed', 'response']) page.on(e, check);
  check();
});

// One route as a first visit: the sorted paths of the built files it fetched, or null when `mayBeGone` and no screen has
// this key (the screen was removed from the app).
async function listRoute(browser, hub, dist, route, horizon, mayBeGone) {
  const context = await browser.newContext({ ...PHONE, baseURL: hub.url });
  try {
    const page = await context.newPage();
    const fetched = new Set(), pending = new Set();
    let started = 0;
    // An EventSource or WebSocket stays open for the life of the page, and so does a response that is an event stream.
    const live = (r) => ['eventsource', 'websocket'].includes(r.resourceType());
    page.on('request', (r) => { started += 1; if (!live(r)) pending.add(r); });
    const done = (r) => pending.delete(r);
    page.on('requestfinished', done);
    page.on('requestfailed', done);
    page.on('response', (res) => {
      if ((res.headers()['content-type'] ?? '').toLowerCase().startsWith('text/event-stream')) pending.delete(res.request());
      const path = res.status() < 400 && builtFile(res.url(), hub.url, dist);
      if (path) fetched.add(path);
    });
    // Nothing the network answered is left to start something more.
    const quiet = async () => {
      for (let again = true; again;) {
        const before = started;
        await drained(page, pending);
        await page.evaluate(() => new Promise((ok) => { const c = new MessageChannel(); c.port1.onmessage = () => ok(); c.port2.postMessage(0); }));
        again = started !== before || pending.size > 0;
      }
    };

    await page.clock.install({ time: CLOCK_START });
    await page.clock.pauseAt(new Date(CLOCK_START.getTime() + 100));
    await page.goto(`/#/${route}`);
    await quiet();
    for (let elapsed = 0; elapsed < horizon; elapsed += STEP_MS) {
      await page.clock.runFor(Math.min(STEP_MS, horizon - elapsed));
      await quiet();
    }
    const state = await page.evaluate(() => ({ path: App.current.path, routed: !!App.current.route, classic: !!(window.LullWebamp || App.classicSettings) }));
    if (!state.routed && mayBeGone) return null;
    assertScreen(route, state);
    if (state.classic) throw new Error(`Classic mode loaded inside the ${horizon} ms of "${route}"'s start: app.js moved its load earlier than the horizon scripts/budgets/list.mjs derives from it`);
    return [...fetched].sort();
  } finally {
    await context.close();
  }
}

// { route: [path, ...] } for every route, each from a first visit of its own; a route in `mayBeGone` whose screen no
// longer exists is null. `horizon` is the start's length in ms of the page's time (horizonOf(app.js)).
export async function listRoutes(chromium, tree, dist, routes, { mayBeGone = [] } = {}) {
  const horizon = horizonOf(readFileSync(join(tree, 'web/app.js'), 'utf8'));
  const hub = await startHub(tree, dist);
  try {
    const browser = await chromium.launch({ headless: true });
    try {
      const out = {};
      for (const route of routes) out[route] = await listRoute(browser, hub, dist, route, horizon, mayBeGone.includes(route));
      return out;
    } finally {
      await browser.close();
    }
  } finally {
    hub.close();
  }
}
