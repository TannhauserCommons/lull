// connect.js on a small stand-in page, for the Connect tests (connect-takeover.test.mjs, connect-device.test.mjs):
// the player records what it is asked, the hub's answers are the test's, and Web Locks are shared between the tabs
// of one test the way a browser shares them.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SRC = readFileSync(join(ROOT, 'web', 'connect.js'), 'utf8');

/** What the page made, as plain data: its arrays and objects come from another realm, which deepEqual tells apart. */
const plain = (x) => JSON.parse(JSON.stringify(x));

/** Every promise callback of the tab has run (the page's promises are all of one or two hops). */
export const settle = async () => { for (let i = 0; i < 8; i++) await new Promise((r) => setImmediate(r)); };

/** The Web Locks of one browser: `tab()` gives a tab its `navigator.locks`, `close()` on it is the tab closing. */
export function browserLocks() {
  const names = {};
  function grant(s) {
    if (s.holder || !s.waiting.length) return;
    const w = s.waiting.shift(), held = (s.holder = w);
    Promise.resolve(w.run()).then((v) => { if (s.holder === held) { s.holder = null; w.resolve(v); grant(s); } });
  }
  return {
    tab() {
      const me = {};
      return {
        request(name, opts, run) {
          if (typeof opts === 'function') { run = opts; opts = {}; }
          const s = (names[name] = names[name] || { holder: null, waiting: [] });
          return new Promise((resolve, reject) => {
            const w = { tab: me, run: () => run({ name }), resolve, reject };
            if (opts.steal) {
              if (s.holder) s.holder.reject(Object.assign(new Error('The lock was stolen'), { name: 'AbortError' }));
              s.holder = null; s.waiting.unshift(w);
            } else s.waiting.push(w);
            grant(s);
          });
        },
        close() {
          for (const s of Object.values(names)) {
            s.waiting = s.waiting.filter((w) => w.tab !== me);
            if (s.holder && s.holder.tab === me) { s.holder = null; grant(s); }
          }
        },
      };
    },
  };
}

/**
 * One tab of the listener's browser, with connect.js loaded. `known`: the song ids this page can read;
 * `items`: the hub's device list; `locks`: this tab's `navigator.locks` (none: a browser without them).
 * `post(path, body)` answers the page's POSTs (a rejection is the hub refusing).
 */
export function connectPage({ known = [], items = [], me = 'me', locks, post } = {}) {
  const store = {}, handlers = {}, adopted = [], toasts = [], posts = [], asked = [];
  const called = (name) => (...args) => { asked.push([name, ...args]); };
  const player = {
    state: { queue: [], index: 0, pos: 0, playing: false, muted: false, volume: 1, shuffle: false, repeat: 'off', radio: null },
    currentId() { const q = this.state.queue[this.state.index]; return q ? q.trackId : null; },
    adopt: (...args) => adopted.push(plain(args)),
    play: called('play'), pause: called('pause'), next: called('next'), prev: called('prev'), seek: called('seek'),
    setVolume: called('volume'), addToQueue: called('enqueue'),
  };
  const App = {
    html: () => '', ui: {}, icon: () => '', $: () => null, action() {}, toast: (m, o) => toasts.push([m, o]),
    server: { authorized: true },
    on(name, fn) { (handlers[name] = handlers[name] || []).push(fn); return () => { handlers[name] = handlers[name].filter((f) => f !== fn); }; },
    store: { subscribe() {}, set: (k, v) => { store[k] = v; }, get: (k) => store[k] },
    data: {
      track: (id) => (known.includes(id) ? { id, duration: 200 } : null),
      api: (path, opts) => {
        if (!opts) return Promise.resolve(path === '/api/devices' ? { items: structuredClone(items), me } : {});
        posts.push([path, plain(opts.body)]);
        return post ? post(path, opts.body) : Promise.resolve({});
      },
    },
    player,
  };
  const unref = (f, ms) => { const t = setTimeout(f, ms); t.unref(); return t; };
  const ctx = vm.createContext({
    App, document: { addEventListener() {} }, addEventListener() {}, navigator: locks ? { locks } : {}, setTimeout: unref, clearTimeout,
    LullRules: { RADIO_STEER: 0.4 }, console, setInterval() {}, Date, Math, JSON, Object, Promise, Error,
  });
  vm.runInContext(SRC, ctx);
  const emit = (name, payload) => (handlers[name] || []).slice().forEach((fn) => fn(payload));
  return {
    App, player, store, adopted, toasts, posts, asked, emit,
    /** the page has met the hub: it reads the device list and reports */
    async start() { emit('server', { authorized: true, checked: true }); await settle(); },
    /** what the other devices heard of this one: the states posted so far */
    reports: () => posts.filter(([path]) => path === '/api/devices/me/state').map(([, body]) => body),
    /** the entries of this page's queue, as the player keeps them */
    queue(entries) { player.state.queue = entries; },
  };
}
