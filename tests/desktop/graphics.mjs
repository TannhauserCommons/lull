// The Linux app's graphics on a computer with no graphics card (GitHub's runners), the real app driven over WebDriver
// (tauri-driver and WebKitGTK's WebKitWebDriver) against the fixture hub (tests/ui/fixture-hub). docs/context/desktop.md,
// Graphics; docs/context/quality.md, Graphics.
// - Each mode (Automatic, Automatic with LIBGL_ALWAYS_SOFTWARE=1, the graphics card started with the switches that turn it
//   off, the processor only) gives the page's process the switches it names, starts and draws (the screen isn't blank),
//   finds the processor drawing (WebKitGTK's own report) and takes the lighter look.
// - A start in a chosen mode that ends before the listener used the page comes back in Automatic, saying so; one the
//   listener used keeps its mode.
// - On the processor, every built-in design holds the frame-rate budget (graphics-budget.json) while long pages scroll,
//   in the look listeners there get: a design that only runs well on a graphics card fails here. The full look's rates
//   are printed beside it.
//
//   node tests/desktop/graphics.mjs <app binary> <identifier>
// Under Xvfb and a D-Bus session, with tauri-driver and WebKitWebDriver on PATH; the binary built with that identifier
// (TAURI_CONFIG), so its settings and logs are its own. LULL_GRAPHICS_ONLY=budget runs the frame rates alone.

import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:net';
import { mkdirSync, readFileSync, readdirSync, readlinkSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

const [binary, identifier] = process.argv.slice(2);
if (!binary || !identifier) throw new Error('usage: node tests/desktop/graphics.mjs <app binary> <identifier>');
const here = dirname(fileURLToPath(import.meta.url));
const ROOT = join(here, '..', '..');
const CONFIG = join(process.env.XDG_CONFIG_HOME || join(homedir(), '.config'), identifier);
const LOG = join(process.env.XDG_DATA_HOME || join(homedir(), '.local', 'share'), identifier, 'logs', 'webview.log');
const BUDGET = JSON.parse(readFileSync(join(here, 'graphics-budget.json'), 'utf8'));
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

const freePort = () => new Promise((ok, fail) => {
  const s = createServer().listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)); }).on('error', fail);
});
async function until(what, fn, ms = 30000) {
  const end = Date.now() + ms;
  for (;;) {
    let last;
    try { last = await fn(); if (last) return last; } catch (e) { last = e; }
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}${last instanceof Error ? ': ' + last.message : ''}`);
    await sleep(250);
  }
}

/* ---------- the fixture hub, as the UI gates start it ---------- */
const hubPort = await freePort();
const hub = spawn(process.execPath, [join(ROOT, 'tests/ui/fixture-hub/server.mjs'), String(hubPort)], { stdio: 'ignore' });
const HUB = `http://127.0.0.1:${hubPort}`;
await until('the fixture hub', async () => (await fetch(HUB + '/health')).ok);

/* ---------- one run of the app: tauri-driver with this environment, a WebDriver session, the app ---------- */
const switches = ['WEBKIT_DISABLE_COMPOSITING_MODE', 'WEBKIT_DISABLE_DMABUF_RENDERER', 'WEBKIT_SKIA_ENABLE_CPU_RENDERING',
  'LIBGL_ALWAYS_SOFTWARE', '__EGL_VENDOR_LIBRARY_FILENAMES', '__NV_PRIME_RENDER_OFFLOAD', 'DRI_PRIME', 'WEBKIT_WEB_RENDER_DEVICE_FILE'];

function settings() { try { return JSON.parse(readFileSync(join(CONFIG, 'settings.json'), 'utf8')); } catch { return {}; } }
function writeSettings(patch) {
  mkdirSync(CONFIG, { recursive: true });
  // paired with the fixture hub, which takes any token; Close quits (no tray on the runner)
  const s = { ...settings(), hub: { mode: 'remote', origin: HUB }, token: 'graphics-check', closeAsked: true, ...patch };
  writeFileSync(join(CONFIG, 'settings.json'), JSON.stringify(s));
}

async function launch(env = {}) {
  const port = await freePort();
  const driver = spawn('tauri-driver', ['--port', String(port)], { env: { ...process.env, ...env }, stdio: ['ignore', 'inherit', 'inherit'] });
  const base = `http://127.0.0.1:${port}`;
  await until('tauri-driver', async () => (await fetch(base + '/status')).ok, 15000);
  const call = async (method, path, body) => {
    // a call that never answers fails the check instead of holding the job
    const r = await fetch(base + path, { method, headers: { 'content-type': 'application/json' }, body: body && JSON.stringify(body), signal: AbortSignal.timeout(60000) });
    const json = await r.json();
    if (!r.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`);
    return json.value;
  };
  const session = await call('POST', '/session', { capabilities: { alwaysMatch: { 'tauri:options': { application: binary } } } });
  const id = session.sessionId;
  const app = {
    // a function's source, called with args; a returned promise is awaited (WebDriver's async script)
    run: (fn, ...args) => call('POST', `/session/${id}/execute/async`, {
      script: `const done = arguments[arguments.length - 1]; Promise.resolve((${fn}).apply(null, Array.from(arguments).slice(0, -1)))
        .then(done, (e) => done({ __error: String(e && e.stack || e) }));`,
      args,
    }).then((v) => { if (v && v.__error) throw new Error(v.__error); return v; }),
    // a real key press (trusted), as the listener's first use of the page
    press: () => call('POST', `/session/${id}/actions`, { actions: [{ type: 'key', id: 'k', actions: [{ type: 'keyDown', value: '' }, { type: 'keyUp', value: '' }] }] }),
    // ended, and gone: lull lets one instance run, and a start beside one still exiting hands over to it and quits
    quit: async () => {
      try { await call('DELETE', `/session/${id}`); } finally { driver.kill(); }
      try { await until('the app to exit', () => appPids().length === 0, 15000); } catch { appPids().forEach((p) => process.kill(+p, 'SIGKILL')); }
    },
  };
  try {
    await until('the hub\'s web UI in the app', () => app.run(() => !!(window.App && App.server && App.server.checked && window.LullDesktop)));
  } catch (e) { await app.quit(); throw e; }
  return app;
}

const exeOf = (p) => { try { return readlinkSync(`/proc/${p}/exe`); } catch { return ''; } };
const pids = () => readdirSync('/proc').filter((p) => /^\d+$/.test(p));
const appPids = () => pids().filter((p) => exeOf(p).endsWith('/' + binary.split('/').pop()));

/* the page's process (WebKitWebProcess) of the app: its environment is what the mode set before it started */
function pageEnvironment() {
  const procs = readdirSync('/proc').filter((p) => /^\d+$/.test(p));
  const exe = (p) => { try { return readlinkSync(`/proc/${p}/exe`); } catch { return ''; } };
  const parent = (p) => { try { return readFileSync(`/proc/${p}/stat`, 'utf8').replace(/^.*\) /, '').split(' ')[1]; } catch { return ''; } };
  const apps = new Set(procs.filter((p) => exe(p).endsWith('/' + binary.split('/').pop())));
  const page = procs.find((p) => exe(p).endsWith('/WebKitWebProcess') && apps.has(parent(p)));
  assert.ok(page, 'the app has a WebKitWebProcess');
  return Object.fromEntries(readFileSync(`/proc/${page}/environ`, 'utf8').split('\0').filter(Boolean)
    .map((kv) => [kv.slice(0, kv.indexOf('=')), kv.slice(kv.indexOf('=') + 1)]));
}

/* the screen (Xvfb), as the listener would see it: not one flat colour */
function screenDrawn() {
  const sd = +execFileSync('convert', ['x:root', '-colorspace', 'gray', '-format', '%[fx:standard_deviation]', 'info:']).toString();
  return sd;
}

const logLines = () => (existsSync(LOG) ? readFileSync(LOG, 'utf8').split('\n') : []);

/* ---------- the modes ---------- */
const MESA_EGL = ['/usr/share/glvnd/egl_vendor.d/50_mesa.json', '/etc/glvnd/egl_vendor.d/50_mesa.json'].find(existsSync);
const MODES = [
  { name: 'Automatic', mode: 'auto', env: {}, want: {} },
  { name: 'Automatic, LIBGL_ALWAYS_SOFTWARE=1', mode: 'auto', env: { LIBGL_ALWAYS_SOFTWARE: '1' }, want: { LIBGL_ALWAYS_SOFTWARE: '1' } },
  // started with the switches a desktop entry or the listener may set: the graphics card clears them
  { name: 'the graphics card', mode: 'gpu', env: { WEBKIT_DISABLE_DMABUF_RENDERER: '1', LIBGL_ALWAYS_SOFTWARE: '1' }, want: {} },
  { name: 'the processor only', mode: 'cpu', env: {}, want: { WEBKIT_DISABLE_COMPOSITING_MODE: '1', WEBKIT_DISABLE_DMABUF_RENDERER: '1',
    WEBKIT_SKIA_ENABLE_CPU_RENDERING: '1', LIBGL_ALWAYS_SOFTWARE: '1', ...(MESA_EGL ? { __EGL_VENDOR_LIBRARY_FILENAMES: MESA_EGL } : {}) } },
];

const results = [];
function check(name, fn) {
  return fn().then(() => { results.push(['ok', name]); console.log(`ok    ${name}`); },
    (e) => { results.push(['FAIL', name]); console.log(`FAIL  ${name}\n      ${String(e.stack || e).split('\n').join('\n      ')}`); });
}
const graphicsState = (app) => app.run(() => window.LullDesktop.graphics.state);
const light = (app) => app.run(() => document.documentElement.getAttribute('data-graphics'));

rmSync(CONFIG, { recursive: true, force: true });
rmSync(dirname(LOG), { recursive: true, force: true });

if (process.env.LULL_GRAPHICS_ONLY !== 'budget') {
  for (const m of MODES) {
    await check(`${m.name}: the engine's switches, a drawn window, the processor found, the lighter look`, async () => {
      writeSettings({ graphics: m.mode, graphicsTrial: null, graphicsFellBack: null, fullLook: false });
      const app = await launch(m.env);
      try {
        await app.press();     // the listener used it: this start's graphics work
        const env = pageEnvironment();
        for (const name of switches) assert.equal(env[name], m.want[name], `${name} in the page's process`);
        const state = await until('what draws the page', async () => { const s = await graphicsState(app); return s && s.software && s; }, 20000);
        assert.equal(state.mode, m.mode);
        assert.equal(state.light, true, 'the lighter look where the processor draws');
        assert.equal(await light(app), 'light', ':root[data-graphics="light"]');
        const drawn = screenDrawn();
        assert.ok(drawn > 0.05, `the window shows lull, not a blank (the screen's grey standard deviation is ${drawn.toFixed(3)})`);
        const said = logLines().filter((l) => l.includes(' graphics: ')).slice(-2);
        assert.ok(said.some((l) => /drawn by the processor/.test(l)), `webview.log says what draws the page:\n${said.join('\n')}`);
        console.log(`      ${said.join('\n      ')}`);
      } finally { await app.quit(); }
    });
  }

  await check('a chosen mode whose start never came up starts again in Automatic, and Settings says why', async () => {
    writeSettings({ graphics: 'gpu', graphicsTrial: null, graphicsFellBack: null });
    let app = await launch();
    await app.quit();                      // shown, never used: a blank window the listener closed
    assert.equal(settings().graphicsTrial, 'gpu', 'the start stays on trial');
    app = await launch();
    try {
      const state = await graphicsState(app) || await until('the graphics state', () => graphicsState(app));
      assert.equal(state.mode, 'auto');
      assert.equal(state.fellBack, 'gpu');
      assert.equal(pageEnvironment().WEBKIT_WEB_RENDER_DEVICE_FILE, undefined);
      await app.run(() => App.go('/settings/app'));
      const line = await until('Settings › App\'s graphics line', () => app.run(() => document.querySelector('#sys-graphics')?.textContent.trim()));
      assert.match(line, /The last start with “Use the graphics card” didn’t come up, so lull went back to Automatic/);
    } finally { await app.quit(); }
  });

  await check('a chosen mode the listener used keeps it', async () => {
    writeSettings({ graphics: 'cpu', graphicsTrial: null, graphicsFellBack: null });
    let app = await launch();
    await app.press();
    await until('the trial ends', () => settings().graphicsTrial == null, 5000);
    await app.quit();
    app = await launch();
    try { assert.equal((await graphicsState(app)).mode, 'cpu'); } finally { await app.quit(); }
  });
}

/* ---------- the frame-rate budget, on the processor ---------- */
// Long pages scrolled a step every frame for `ms`: the frames the page drew a second (its requestAnimationFrame callbacks,
// which wait for each paint). The scroller must move, or nothing was drawn.
async function scrollRate(app, ms) {
  return app.run((ms, step) => new Promise((done) => {
    const el = [document.querySelector('.main'), ...document.querySelectorAll('#view, #view *')]
      .find((e) => e && e.scrollHeight > e.clientHeight + 200 && /auto|scroll/.test(getComputedStyle(e).overflowY));
    if (!el) { done({ fps: 0, moved: 0, why: 'nothing scrolls' }); return; }
    let frames = 0, dir = 1, moved = 0, start;
    const tick = (t) => {
      if (start === undefined) start = t; else frames++;
      const before = el.scrollTop;
      el.scrollTop += step * dir;
      moved += Math.abs(el.scrollTop - before);
      if (el.scrollTop + el.clientHeight >= el.scrollHeight - 1) dir = -1; else if (el.scrollTop <= 0) dir = 1;
      if (t - start < ms) requestAnimationFrame(tick); else done({ fps: frames * 1000 / (t - start), moved });
    };
    requestAnimationFrame(tick);
  }), ms, BUDGET.stepPx);
}
async function settle(app) {
  await app.run(() => document.fonts.ready.then(() => true));
  let last = '';
  for (let i = 0, stable = 0; i < 60 && stable < 4; i++) {
    await sleep(100);
    const now = await app.run(() => `${document.querySelector('#view')?.innerHTML.length}:${document.images.length}`);
    stable = now === last ? stable + 1 : 0;
    last = now;
  }
}

// The page's process's CPU time (/proc/<pid>/stat: user + system, in clock ticks of 10 ms).
function pageCpu() {
  const procs = readdirSync('/proc').filter((p) => /^\d+$/.test(p));
  const exe = (p) => { try { return readlinkSync(`/proc/${p}/exe`); } catch { return ''; } };
  return procs.filter((p) => exe(p).endsWith('/WebKitWebProcess'))
    .reduce((sum, p) => { const f = readFileSync(`/proc/${p}/stat`, 'utf8').replace(/^.*\) /, '').split(' '); return sum + +f[11] + +f[12]; }, 0) * 10;
}
// The median of `runs` measurements: one held up by the runner doesn't decide.
const median = (xs) => xs.slice().sort((a, b) => a - b)[Math.floor(xs.length / 2)];

// Each design and route, warmed first (built once, its covers in, scrolled through once), then measured in each look;
// and Home at rest in each look: the share of a core the page's process takes.
const rates = [], rest = [];
async function measure(app) {
  const failures = [];
  for (const design of DESIGNS) {
    for (const route of BUDGET.routes) {
      await app.run((d, r) => { localStorage.setItem('lull:prefs', JSON.stringify({ ...JSON.parse(localStorage.getItem('lull:prefs') || '{}'), design: d })); App.setDesign(d); App.go(r); return true; }, design, route);
      await settle(app);
      await scrollRate(app, BUDGET.warmMs);
      for (const look of ['light', 'full']) {
        await app.run((full) => window.LullDesktop.setSettings({ fullLook: full }), look === 'full');
        await until(`the ${look} look`, async () => (await light(app)) === (look === 'light' ? 'light' : null), 5000);
        await sleep(300);
        if (route === BUDGET.routes[0]) {
          await sleep(BUDGET.quietMs);    // the switch's own repaint, done
          const before = pageCpu(); await sleep(BUDGET.restMs);
          const core = (pageCpu() - before) / BUDGET.restMs;
          rest.push({ look, design, core });
          const known = KNOWN.rest.find((x) => x.design === design);
          if (look === 'light' && core > BUDGET.restMaxCore && !known) failures.push(`${design} at rest: ${(core * 100).toFixed(0)}% of a core`);
          if (look === 'light' && known && core <= BUDGET.restMaxCore) cleared.push(`${design} at rest`);
        }
        const runs = [];
        for (let k = 0; k < BUDGET.runs; k++) runs.push(await scrollRate(app, BUDGET.ms));
        const fps = median(runs.map((r) => r.fps));
        rates.push({ look, design, route, fps });
        const known = KNOWN.pages.find((x) => x.design === design && x.route === route);
        if (look === 'light' && (runs.some((r) => r.moved <= 0) || fps < BUDGET.minFps) && !known) {
          failures.push(`${design} ${route}: ${fps.toFixed(1)} frames a second${runs[0].why ? ' (' + runs[0].why + ')' : ''}`);
        }
        if (look === 'light' && known && fps >= BUDGET.minFps) cleared.push(`${design} ${route}`);
      }
    }
  }
  return failures;
}
const KNOWN = JSON.parse(readFileSync(join(here, 'graphics-known.json'), 'utf8'));
const cleared = [];
await check(`on the processor, in the lighter look, every design scrolls long pages at ${BUDGET.minFps}+ frames a second and rests under ${BUDGET.restMaxCore * 100}% of a core`, async () => {
  writeSettings({ graphics: 'auto', graphicsTrial: null, graphicsFellBack: null, fullLook: false });
  const app = await launch();
  try {
    await app.press();
    await until('the lighter look', async () => (await light(app)) === 'light', 20000);
    assert.deepEqual(await measure(app), [], 'under the budget, and not in graphics-known.json');
    // the list only shrinks: one that held the budget this time is named, for its owner to take off once it holds it
    // steadily (a single run is too noisy to fail on)
    for (const c of cleared) console.log(`::notice::${c} held the graphics budget this time: take it off tests/desktop/graphics-known.json once it does every time`);
  } finally { await app.quit(); }
});

const table = ['| design | route | lighter look | full look |', '|---|---|---|---|'];
for (const design of DESIGNS) for (const route of BUDGET.routes) {
  const f = (look) => { const r = rates.find((x) => x.look === look && x.design === design && x.route === route); return r ? r.fps.toFixed(1) : '-'; };
  const known = KNOWN.pages.some((x) => x.design === design && x.route === route) ? ' (known)' : '';
  table.push(`| ${design} | ${route} | ${f('light')}${known} | ${f('full')} |`);
}
const restTable = ['| design | lighter look | full look |', '|---|---|---|'];
for (const design of DESIGNS) {
  const f = (look) => { const r = rest.find((x) => x.look === look && x.design === design); return r ? `${(r.core * 100).toFixed(0)}% of a core` : '-'; };
  restTable.push(`| ${design} | ${f('light')}${KNOWN.rest.some((x) => x.design === design) ? ' (known)' : ''} | ${f('full')} |`);
}
const said = ['Frames a second while scrolling, on the processor (Xvfb, Mesa llvmpipe), the median of ' + BUDGET.runs + ':', ...table, '',
  `Home at rest (${BUDGET.restMs / 1000} s), the page's process:`, ...restTable];
console.log('\n' + said.join('\n'));
if (process.env.GITHUB_STEP_SUMMARY) {
  writeFileSync(process.env.GITHUB_STEP_SUMMARY, ['### The app on the processor', '',
    `Budget, in the lighter look: ${BUDGET.minFps} frames a second scrolling, ${BUDGET.restMaxCore * 100}% of a core at rest (tests/desktop/graphics-budget.json); what graphics-known.json lists misses it on main.`, '', ...said].join('\n') + '\n', { flag: 'a' });
}

hub.kill();
const failed = results.filter(([s]) => s === 'FAIL');
console.log(`\n${results.length - failed.length} passed, ${failed.length} failed`);
process.exit(failed.length ? 1 : 0);
