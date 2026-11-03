// The desktop app's main window tells its page when it is out of sight (platform::set_webview_awake: WebView2's
// IsVisible and a low memory target), so the page's hidden-document rules apply: loops pause (:root.is-hidden), screen
// ticks stop, prefetching waits. Run it against a debug build under its own identifier with DevTools open
// (docs/context/desktop.md, Run it and Inspecting the webviews), signed in to a hub so main shows the web UI:
//
//   node tests/desktop/visibility.mjs [port]          (default 9333)
//
// It opens the mini player (main hides), brings main back, minimises it and brings it back, and reads the page's own
// visibility each time over a bare DevTools socket (an automation library attached to the page can hold it "visible");
// with a song loaded it also plays it out of sight and checks it goes on.
// Exits 1 on any mismatch. Node 22 or later (WebSocket and fetch built in).

const port = +(process.argv[2] || 9333);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

let main;
for (let i = 0; i < 60 && !main; i++) {
  try {
    const targets = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
    main = targets.find((t) => t.type === 'page' && /^https?:/.test(t.url) && !/\/desktop\//.test(t.url));
  } catch { /* the app is still starting */ }
  if (!main) await sleep(500);
}
if (!main) { console.error(`no main window with the web UI on port ${port}`); process.exit(1); }

const ws = new WebSocket(main.webSocketDebuggerUrl);
await new Promise((ok, fail) => { ws.addEventListener('open', ok, { once: true }); ws.addEventListener('error', fail, { once: true }); });
let next = 0;
const waiting = new Map();
ws.addEventListener('message', (m) => { const d = JSON.parse(m.data); if (waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); } });
const evaluate = (expression) => new Promise((ok) => {
  const id = ++next;
  waiting.set(id, (d) => ok(d.result?.result?.value));
  ws.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
});

await evaluate(`window.__seen = []; if (!window.__seenOn) { window.__seenOn = true; document.addEventListener('visibilitychange', () => window.__seen.push(document.visibilityState)); } true`);
const steps = [
  ['the mini player open (main hidden)', `LullDesktop.openMini()`, 'hidden'],
  ['main back', `LullDesktop.windowAction('full')`, 'visible'],
  ['main minimised', `window.__TAURI__.window.getCurrentWindow().minimize()`, 'hidden'],
  ['main back again', `LullDesktop.windowAction('full')`, 'visible'],
];
let failed = 0;
for (const [what, action, want] of steps) {
  await evaluate(`Promise.resolve(${action}).then(() => true, () => false)`);
  await sleep(2500);
  const got = await evaluate('document.visibilityState');
  const ok = got === want;
  if (!ok) failed++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}: ${got}${ok ? '' : ` (want ${want})`}`);
}
// and playback goes on out of sight: an audible page isn't throttled (a song loaded, as when it was left)
if (await evaluate('!!(window.App && App.player.current())')) {
  await evaluate('App.player.play()');
  await sleep(1500);
  await evaluate('LullDesktop.openMini()');
  await sleep(1000);
  const from = await evaluate('App.player.state.pos');
  await sleep(3000);
  const to = await evaluate('App.player.state.pos'), hidden = await evaluate('document.visibilityState');
  const ok = hidden === 'hidden' && to - from > 2;
  if (!ok) failed++;
  console.log(`${ok ? 'ok  ' : 'FAIL'} playing while hidden: ${hidden}, the song went on ${(to - from).toFixed(1)} s in 3 s`);
  await evaluate('App.player.pause()');
  await evaluate("LullDesktop.windowAction('full')");
  await sleep(1500);
} else console.log('skip playing while hidden: no song loaded');
console.log('visibilitychange events:', (await evaluate('JSON.stringify(window.__seen)')) || '[]');
ws.close();
process.exit(failed ? 1 : 0);
