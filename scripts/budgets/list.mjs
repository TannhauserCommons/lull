// The files a first load of a route fetches, as a real browser lists them: `listRoutes(chromium, tree, dist, routes)`.
//
// The tree's fixture hub (tests/ui/fixture-hub: no Soulseek, no outside service, the same answers every run) serves the
// tree's built web files, and Chromium opens each route as a first visit would: a fresh context for every route, Lighthouse's
// phone (412x823 at 1.75x), light mode, the default design, no stored settings, no service worker. Every same-origin request
// whose path is a file of the built tree is listed (documents, scripts, styles, fonts, images and files fetched from script
// alike); the hub's own answers (/api, /health) are not files of the tree.
//
// A start is what has loaded by the time the page has settled: it has booted its design, drawn the route's screen, and a
// round of "nothing in flight, then the browser idle" starts no new request. What lull loads later on purpose is outside
// it: Classic mode about 15 s after the start (app.js), the UI sounds after a first tap or key (App.needSounds), and
// anything a click opens. The listing fails if settling takes long enough to reach the first of those.
//
// Settling waits on events, never a sleep. The page's own handlers run after the browser has reported a response finished (a
// script's `load` is a task of its own, and what it fetches starts then), so "nothing pending" can be true a moment before
// the next request: a round therefore ends with a message through a MessageChannel (a task queued behind those handlers)
// and an idle callback (the browser has no task left), and the listing is settled only when two rounds in a row see no
// request begin and none in flight.
import { spawn } from 'node:child_process';
import { existsSync, statSync } from 'node:fs';
import { createServer } from 'node:net';
import { join, posix } from 'node:path';

// Lighthouse's mobile profile as docs/context/quality.md describes it, in light mode.
export const PHONE = {
  viewport: { width: 412, height: 823 }, deviceScaleFactor: 1.75, isMobile: true, hasTouch: true,
  serviceWorkers: 'block', locale: 'en-GB', timezoneId: 'UTC', colorScheme: 'light',
};
// The earliest of the loads outside the start is Classic's, 15 s after the page is ready.
export const SETTLE_LIMIT_MS = 10_000;

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
  return new Promise((ok, fail) => {
    const s = createServer().listen(0, '127.0.0.1', () => { const { port } = s.address(); s.close(() => ok(port)); }).on('error', fail);
  });
}

// The tree's own fixture hub on a free port, serving `dist`; `close()` stops it.
export async function startHub(tree, dist) {
  const port = await freePort();
  const server = join(tree, 'tests/ui/fixture-hub/server.mjs');
  const child = spawn(process.execPath, [server, String(port)], { env: { ...process.env, LULL_WEB: dist }, stdio: 'ignore' });
  const url = `http://127.0.0.1:${port}`;
  for (let i = 0; ; i++) {
    try { if ((await fetch(`${url}/health`)).ok) break; } catch { /* not listening yet */ }
    if (child.exitCode !== null || i > 200) { child.kill(); throw new Error(`the fixture hub did not start on ${url}`); }
    await new Promise((ok) => setTimeout(ok, 50));
  }
  return { url, close: () => child.kill() };
}

// Resolves once nothing tracked is in flight, on the request events themselves.
const drained = (page, pending) => new Promise((resolve) => {
  const check = () => {
    if (pending.size) return;
    page.off('requestfinished', check);
    page.off('requestfailed', check);
    resolve();
  };
  page.on('requestfinished', check);
  page.on('requestfailed', check);
  check();
});

// One route as a first visit: the sorted paths of the built files it fetched.
async function listRoute(browser, hub, dist, route) {
  const context = await browser.newContext({ ...PHONE, baseURL: hub.url });
  try {
    const page = await context.newPage();
    const fetched = new Set(), pending = new Set();
    let started = 0;
    // Live streams (events, a socket) stay open for the life of the page and are never "in flight".
    const tracked = (r) => !['eventsource', 'websocket'].includes(r.resourceType());
    page.on('request', (r) => { started += 1; if (tracked(r)) pending.add(r); });
    const done = (r) => pending.delete(r);
    page.on('requestfinished', done);
    page.on('requestfailed', done);
    page.on('response', (res) => {
      const path = res.status() < 400 && builtFile(res.url(), hub.url, dist);
      if (path) fetched.add(path);
    });

    await page.goto(`/#/${route}`);
    await page.waitForFunction(() => window.App && App.server && App.server.checked);
    await page.evaluate(() => new Promise((ok) => (window.Designs ? Designs.ready(ok) : ok())));
    await page.waitForFunction(() => document.querySelector('#view')?.firstElementChild);
    for (let quiet = 0; quiet < 2;) {
      const before = started;
      await page.evaluate(() => document.fonts.ready);
      await drained(page, pending);
      await page.evaluate(() => new Promise((ok) => {
        const channel = new MessageChannel();
        channel.port1.onmessage = () => requestIdleCallback(() => ok(), { timeout: 1000 });
        channel.port2.postMessage(0);
      }));
      quiet = started === before && !pending.size ? quiet + 1 : 0;
    }
    const state = await page.evaluate(() => ({ path: App.current.path, routed: !!App.current.route, ms: performance.now() }));
    assertScreen(route, state);
    if (state.ms > SETTLE_LIMIT_MS) throw new Error(`the page took ${Math.round(state.ms)} ms to settle on "${route}": too close to Classic's load at 15 s to say what a start is`);
    return [...fetched].sort();
  } finally {
    await context.close();
  }
}

// { route: [path, ...] } for every route, each from a first visit of its own.
export async function listRoutes(chromium, tree, dist, routes) {
  const hub = await startHub(tree, dist);
  const browser = await chromium.launch({ headless: true });
  try {
    const out = {};
    for (const route of routes) out[route] = await listRoute(browser, hub, dist, route);
    return out;
  } finally {
    await browser.close();
    hub.close();
  }
}
