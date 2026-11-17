// "Play here" (web/connect.js C.takeover): the other device's queue continues on this one at the entry it was playing,
// even when that song is in the queue twice, and when songs this device can't read are left out of the list before it.
// connect.js runs in a small stand-in page, with the player's playList recorded.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SRC = readFileSync(join(ROOT, 'web', 'connect.js'), 'utf8');

/* Connect on a page that knows `known`; the other device ("d1") reports `queue` playing the entry at `queueIndex` */
async function takeover(known, queue, queueIndex) {
  const store = {}, handlers = {}, played = [], toasts = [];
  const device = { id: 'd1', name: 'Kitchen', state: { trackId: queue[queueIndex], queue, queueIndex, position: 12, playing: true, recv: Date.now() } };

  const App = {
    html: () => '', ui: {}, icon: () => '', $: () => null, action() {}, toast: (m, o) => toasts.push([m, o]),
    on(name, fn) { (handlers[name] = handlers[name] || []).push(fn); return () => { handlers[name] = handlers[name].filter((f) => f !== fn); }; },
    store: { subscribe() {}, set: (k, v) => { store[k] = v; }, get: (k) => store[k] },
    data: { track: (id) => (known.includes(id) ? { id, duration: 200 } : null), api: () => Promise.resolve({}) },
    player: { state: {}, currentId() {}, playList: (...args) => played.push(args) },
  };
  const ctx = vm.createContext({ App, document: { addEventListener() {} }, setTimeout, clearTimeout, setInterval() {}, Date, Math, JSON, Object, Promise });
  vm.runInContext(SRC, ctx);
  store.devices = { items: [device], me: 'me' };   // after connect.js has set its own empty list
  const done = App.connect.takeover(device);
  (handlers['server:device.state'] || []).forEach((fn) => fn({ deviceId: 'd1', ...device.state }));
  await done
  return { played, toasts };
}

test('Play here starts at the entry the other device was on, not the first copy of that song', async () => {
  const { played } = await takeover(['a', 'b', 'c'], ['a', 'b', 'a', 'c'], 2);
  assert.equal(played.length, 1);
  assert.deepEqual(played[0][0], ['a', 'b', 'a', 'c']);
  assert.equal(played[0][1], 2);
});

test('Play here leaves out the songs this device can\'t read and still starts at the entry the other device was on', async () => {
  const { played } = await takeover(['a', 'b', 'c'], ['x', 'a', 'y', 'b', 'a', 'c'], 4);
  assert.deepEqual(played[0][0], ['a', 'b', 'a', 'c']);
  assert.equal(played[0][1], 2);
});

test('Play here says so when the song the other device was playing can\'t be read', async () => {
  const { played, toasts } = await takeover(['a', 'b'], ['a', 'x', 'b'], 1);
  assert.equal(played.length, 0);
  assert.match(toasts[0][0], /Couldn't read the song Kitchen was playing/);
});
