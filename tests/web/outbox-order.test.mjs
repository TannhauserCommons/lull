// The offline outbox (web/offline.js) keeps changes in the order they were made: a change made while older ones
// wait, or while a flush is sending them, queues behind them instead of overtaking them to the hub.
// offline.js runs in a small stand-in page with an in-memory IndexedDB and a hub that records what reaches it.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SRC = readFileSync(join(ROOT, 'web', 'offline.js'), 'utf8');

/* just enough IndexedDB: stores keyed by keyPath (or an automatic seq), transactions that finish on a later tick */
function fakeIndexedDB() {
  const stores = {};
  const keyPaths = {};
  const db = {
    createObjectStore(name, { keyPath }) { stores[name] = new Map(); keyPaths[name] = keyPath; },
    transaction(name) {
      const t = {};
      stores[name].seq = stores[name].seq || 0;
      const req = { result: undefined };
      const store = {
        add(rec) { const k = keyPaths[name] === 'seq' ? ++stores[name].seq : rec[keyPaths[name]]; stores[name].set(k, { ...rec, [keyPaths[name]]: k }); return req; },
        put(rec) { stores[name].set(rec[keyPaths[name]], rec); return req; },
        delete(k) { stores[name].delete(k); return req; },
        getAll() { req.result = [...stores[name].entries()].sort((a, b) => a[0] - b[0]).map((e) => e[1]); return req; },
      };
      t.objectStore = () => store;
      setTimeout(() => t.oncomplete && t.oncomplete(), 0);
      return t;
    },
  };
  return {
    open() {
      const req = {};
      setTimeout(() => { req.result = db; req.onupgradeneeded(); req.onsuccess(); }, 0);
      return req;
    },
  };
}

const tick = (ms = 5) => new Promise((r) => setTimeout(r, ms));
/* wait for the queue to drain by itself; fails the test, not hangs, if it never does */
async function drained(App) {
  for (let i = 0; i < 200 && App.outbox.size() > 0; i++) await tick(10);
  assert.equal(App.outbox.size(), 0, 'the outbox emptied');
}

/* the page, the hub's answers and the outbox; `hub.down` makes /api/sync fail the way an unreachable hub does */
function page() {
  const hub = { arrived: [], down: false, hold: null };
  const handlers = {};
  const App = {
    html: () => '', ui: {}, icon: () => '', $$: () => [], action() {}, toast() {}, emit() {}, go() {}, refresh() {}, reloadCollections() {},
    current: { path: '/' },
    server: { reachable: true, authorized: true, checked: true },
    on(name, fn) { (handlers[name] = handlers[name] || []).push(fn); },
    store: { subscribe() {}, get: () => ({}) },
    data: {
      absorb() {}, invalidate() {}, track: () => null, album: () => null, playlist: () => null, url: (p) => p,
      /* a like or unlike sent straight to the hub, or a batch sent to /api/sync; both record what arrived, in order */
      async api(path, opts) {
        if (hub.down) { const e = new Error('no answer'); e.status = 0; throw e; }
        if (path === '/api/sync') {
          if (hub.hold) await hub.hold;
          opts.body.ops.forEach((o) => hub.arrived.push(`${o.op}:${o.trackId}:${o.liked ? 'on' : 'off'}`));
          return { applied: opts.body.ops.map((o) => o.id), rejected: [], playlists: {} };
        }
        hub.arrived.push(`${opts.method}:${path}`);
        return {};
      },
    },
  };
  const window = { indexedDB: fakeIndexedDB(), addEventListener() {}, isSecureContext: true };
  const ctx = vm.createContext({
    App, window, indexedDB: window.indexedDB, navigator: { onLine: true }, crypto: { randomUUID: () => 'op-' + Math.random() }, setTimeout, Date, Math, JSON, Object, Promise, Array, Map, URL, console,
  });
  vm.runInContext(SRC, ctx);
  const like = (id, on) => App.outbox.call('/api/likes/' + id, { method: on ? 'PUT' : 'DELETE' }, { op: 'like', trackId: id, liked: on });
  return { App, hub, like };
}

test('a like made offline reaches the hub before the unlike made right after the hub came back', async () => {
  const { App, hub, like } = page();
  hub.down = true;
  App.server.reachable = false;
  await like('t1', true);
  hub.down = false;
  App.server.reachable = true;            // the hub answers again, but the outbox has not flushed yet
  await like('t1', false);
  await drained(App);
  assert.deepEqual(hub.arrived, ['like:t1:on', 'like:t1:off']);
});

test('a like made while the hub can\'t be reached is not overtaken once the hub answers again', async () => {
  const { App, hub, like } = page();
  hub.down = true;                         // the hub looks reachable but doesn't answer: the first like queues
  await like('t1', true);
  hub.down = false;
  await like('t1', false);                 // older change still waiting: this one queues behind it
  await drained(App);
  assert.deepEqual(hub.arrived, ['like:t1:on', 'like:t1:off']);
});

test('a change made while a flush is sending goes out after it, in order', async () => {
  const { App, hub, like } = page();
  hub.down = true;
  App.server.reachable = false;
  await like('t1', true);
  hub.down = false;
  App.server.reachable = true;
  let release;
  hub.hold = new Promise((r) => { release = r; });
  const flush = App.outbox.flush();        // sending the first like, held at the hub
  await tick();
  const unlike = like('t1', false);        // made while the flush is in flight
  release();
  await flush;
  await unlike;
  await drained(App);
  assert.deepEqual(hub.arrived, ['like:t1:on', 'like:t1:off']);
});

test('with nothing waiting a like goes straight to the hub', async () => {
  const { App, hub, like } = page();
  await like('t1', true);
  assert.deepEqual(hub.arrived, ['PUT:/api/likes/t1']);
  assert.equal(App.outbox.size(), 0);
});
