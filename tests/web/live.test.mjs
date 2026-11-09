// web/designs/live.js, the live model a theme names values of (ADR-0101, docs/themes.md section 14), in a small
// stand-in page: what it publishes and where, that it publishes only what is named, and that the clock wakes at the
// minute's edge (the second's only when clock.second is named) and never while the page is hidden.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import vm from 'node:vm';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const SRC = readFileSync(join(ROOT, 'web', 'designs', 'live.js'), 'utf8');
const MODEL_SRC = readFileSync(join(ROOT, 'web', 'designs', 'model.js'), 'utf8');
const MODEL = JSON.parse(readFileSync(join(ROOT, 'web', 'designs', 'live.json'), 'utf8')).values;

/* an element: attributes, a style with custom properties, text, and the selectors live.js asks for */
function el(attrs = {}) {
  const props = {};
  return {
    attrs: { ...attrs }, kids: [], firstChild: null, textContent: '',
    getAttribute(n) { return n in this.attrs ? this.attrs[n] : null; },
    setAttribute(n, v) { this.attrs[n] = String(v); },
    removeAttribute(n) { delete this.attrs[n]; },
    style: { getPropertyValue: (n) => props[n] || '', setProperty: (n, v) => { props[n] = v; }, props },
    querySelectorAll(sel) { return match(this.kids, sel); },
  };
}
function match(list, sel) {
  return list.filter((e) => sel.split(',').some((s) => {
    s = s.trim();
    const bind = /^\[data-bind="(.+)"\]$/.exec(s);
    if (bind) return e.attrs['data-bind'] === bind[1];
    if (s === '[data-bind]') return 'data-bind' in e.attrs;
    if (s === '[data-live]') return 'data-live' in e.attrs;
    if (s === '[data-region]') return 'data-region' in e.attrs;
    if (s.startsWith('#')) return e.attrs.id === s.slice(1);
    return false;
  }));
}

/* a page at a given time: its root, its elements, its timers and its visibility */
function page(at, kids = []) {
  const timers = [];
  const root = el();
  const doc = {
    documentElement: root, hidden: false, listeners: {},
    querySelectorAll: (sel) => match(kids, sel),
    addEventListener(n, fn) { (this.listeners[n] = this.listeners[n] || []).push(fn); },
  };
  let now = at.getTime();
  class FakeDate extends Date { constructor(...a) { super(...(a.length ? a : [now])); } static now() { return now; } }
  const ctx = {
    document: doc, Date: FakeDate, Intl, Object, Array, String, Math, JSON,
    MutationObserver: class { observe() {} takeRecords() { return []; } },
    setTimeout: (fn, ms) => { timers.push({ fn, ms }); return timers.length; },
    clearTimeout: () => {},
    CSS: { registered: [], registerProperty(p) { this.registered.push(p); } },
  };
  ctx.window = ctx;
  vm.createContext(ctx);
  vm.runInContext(MODEL_SRC, ctx);
  vm.runInContext(SRC, ctx);
  return { Live: ctx.Live, root, doc, timers, ctx, kids, advance(ms) { now += ms; }, at: (ms) => { now = ms; } };
}

test('live.js carries every value of web/designs/live.json, with its type and pace', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 30));
  assert.deepEqual(Object.keys(p.Live.model).sort(), Object.keys(MODEL).sort());
  for (const [path, v] of Object.entries(MODEL)) assert.deepEqual([...p.Live.model[path]].slice(0, 2), [v.type, v.pace], path);
});

test('a named value is published in every form its type has: words, the hosts\' property, the root state', () => {
  const words = el({ 'data-bind': 'clock.hour' }), lcd = el({ 'data-bind': 'clock.daypart' });
  const host = el({ 'data-region': 'header' }), mine = el({ 'data-live': '' }), other = el({ 'data-bind': 'clock.minute' });
  const p = page(new Date(2026, 10, 8, 7, 5, 30), [words, lcd, host, mine, other]);
  p.Live.use(['clock.hour', 'clock.daypart']);
  assert.equal(words.textContent, '07');
  assert.equal(lcd.textContent, 'morning');
  assert.equal(host.style.props['--clock-hour'], '7');
  assert.equal(mine.style.props['--clock-daypart'], '"morning"');
  assert.equal(p.root.attrs['data-clock-daypart'], 'morning');
  assert.equal(p.root.attrs['data-clock-hour'], undefined, 'a number is no root state');
  // not named: never computed or written
  assert.equal(other.textContent, '');
  assert.equal(host.style.props['--clock-minute'], undefined);
  // its property is registered, so it doesn't inherit
  assert.deepEqual(p.ctx.CSS.registered.map((r) => [r.name, r.inherits]).sort(), [['--clock-daypart', false], ['--clock-hour', false]]);
});

test('the dayparts turn at 5, 12, 18 and 22, as Home\'s greeting does', () => {
  for (const [h, want] of [[4, 'night'], [5, 'morning'], [11, 'morning'], [12, 'afternoon'], [17, 'afternoon'], [18, 'evening'], [21, 'evening'], [22, 'night'], [0, 'night']]) {
    const p = page(new Date(2026, 10, 8, h, 0, 0));
    p.Live.use(['clock.daypart']);
    assert.equal(p.root.attrs['data-clock-daypart'], want, `${h}:00`);
  }
});

test('the clock wakes at the next minute\'s edge, and at the second\'s only when clock.second is named', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 30, 250));
  p.Live.use(['clock.minute']);
  assert.equal(p.timers.length, 1);
  assert.ok(p.timers[0].ms >= 29750 && p.timers[0].ms <= 29800, `woke after ${p.timers[0].ms} ms`);
  const q = page(new Date(2026, 10, 8, 14, 12, 30, 250));
  q.Live.use(['clock.second']);
  assert.ok(q.timers[0].ms >= 750 && q.timers[0].ms <= 800, `woke after ${q.timers[0].ms} ms`);
});

test('a minute later the words change in place; nothing runs while hidden, and it catches up once shown', () => {
  const words = el({ 'data-bind': 'clock.minute' });
  const p = page(new Date(2026, 10, 8, 14, 12, 59, 900), [words]);
  p.Live.use(['clock.minute']);
  assert.equal(words.textContent, '12');
  p.advance(200);
  p.timers.at(-1).fn();
  assert.equal(words.textContent, '13');
  const armed = p.timers.length;
  p.doc.hidden = true;
  p.doc.listeners.visibilitychange.forEach((fn) => fn());
  p.advance(5 * 60000);
  p.timers.at(-1).fn();          // a timer set before hiding fires: it publishes and arms nothing more
  assert.equal(p.timers.length, armed, 'no timer armed while hidden');
  p.doc.hidden = false;
  p.doc.listeners.visibilitychange.forEach((fn) => fn());
  assert.equal(words.textContent, '18', 'caught up once on showing');
  assert.equal(p.timers.length, armed + 1);
});

test('nothing named: no timer, nothing written; and a value no longer named leaves :root', () => {
  const host = el({ 'data-region': 'header' });
  const p = page(new Date(2026, 10, 8, 14, 12, 0), [host]);
  p.Live.use([]);
  assert.equal(p.timers.length, 0);
  p.Live.use(['clock.daypart']);
  assert.equal(p.root.attrs['data-clock-daypart'], 'afternoon');
  p.Live.use([]);
  assert.equal(p.root.attrs['data-clock-daypart'], undefined);
});

test('a slot drawn between changes shows the value now: its bindings and its hosts are filled as it is drawn', () => {
  const p = page(new Date(2026, 10, 8, 9, 41, 0));
  p.Live.use(['clock.time', 'clock.minute']);
  const box = el(), t = el({ 'data-bind': 'clock.time' }), hand = el({ 'data-live': '' });
  box.kids.push(t, hand);
  p.Live.fill(box);
  assert.match(t.textContent, /9:41|09:41/);
  assert.equal(hand.style.props['--clock-minute'], '41');
  assert.match(p.Live.text('clock.time'), /9:41|09:41/);
  assert.equal(p.Live.text('clock.nope'), '');
});

test('song and player values: words, flags and states on :root, and percentages as words', () => {
  const title = el({ 'data-bind': 'song.title' }), pos = el({ 'data-bind': 'song.pos' }), host = el({ 'data-region': 'playerbar' });
  const p = page(new Date(2026, 10, 8, 14, 12, 0), [title, pos, host]);
  p.Live.use(['song.title', 'song.pos', 'song.liked', 'player.repeat', 'clock.hour']);
  p.Live.feed({ 'song.title': 'Kasagumo', 'song.pos': 0.42, 'song.liked': true, 'player.repeat': 'one', 'song.artist': 'not named' });
  assert.equal(title.textContent, 'Kasagumo');
  assert.equal(pos.textContent, '42%');
  assert.equal(host.style.props['--song-pos'], '0.42');
  assert.equal(host.style.props['--song-title'], '"Kasagumo"');
  assert.equal(host.style.props['--song-liked'], '1');
  assert.equal(p.root.attrs['data-song-liked'], 'true');
  assert.equal(p.root.attrs['data-player-repeat'], 'one');
  assert.equal(host.style.props['--song-artist'], undefined, 'a value nobody names is never written');
});

test('the mini player windows get every named value but the clock\'s and the regions\', and set them as the page has them', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 0));
  p.Live.use(['song.title', 'next.title', 'clock.hour', 'region.width']);
  p.Live.feed({ 'song.title': 'Kasagumo', 'next.title': 'Humid blue' });
  assert.deepEqual(JSON.parse(JSON.stringify(p.Live.values())), { 'song.title': 'Kasagumo', 'next.title': 'Humid blue' });
  const words = el({ 'data-bind': 'next.title' });
  const mini = page(new Date(2026, 10, 8, 14, 12, 0), [words]);
  mini.Live.use(['song.title', 'next.title']);
  mini.Live.feed(p.Live.values());
  assert.equal(words.textContent, 'Humid blue');
});

test('a state with nothing to say is no attribute', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 0));
  p.Live.use(['app.screen']);
  p.Live.feed({ 'app.screen': 'home' });
  assert.equal(p.root.attrs['data-app-screen'], 'home');
  p.Live.feed({ 'app.screen': '' });
  assert.equal(p.root.attrs['data-app-screen'], undefined);
});

test('no script builds data-* names from a bare "data-" prefix, so the hook check still knows which attributes are lull\'s', () => {
  const hooks = readFileSync(join(ROOT, 'web', 'designs', 'hooks.js'), 'utf8');
  const prefixes = JSON.parse(/"prefixes":(\[[^\]]*\])/.exec(hooks)[1]);
  assert.ok(!prefixes.includes('data-'), 'web/designs/hooks.js lists "data-" as a prefix lull builds names from');
});

/* a stand-in App: one song playing from an album, from a Soulseek peer, two songs queued by the listener and one from
   the album after it, and a radio adding more */
function withApp(p) {
  const track = { id: 't1', title: 'Kasagumo', artist: 'A', artistId: 'ar', album: 'Al', albumId: 'al', no: 4, duration: 225, tags: ['city pop'], saved: false, sizeBytes: 30200000 };
  const tracks = { t1: track, a: { ...track, id: 'a', title: 'Two' }, b: { ...track, id: 'b', title: 'Three' }, c: { ...track, id: 'c', title: 'Four' } };
  const state = { queue: [{ trackId: 't1' }, { trackId: 'a', group: 'next' }, { trackId: 'b', group: 'next' }, { trackId: 'c', group: 'context' }],
    index: 0, status: 'streaming', playing: true, shuffle: false, repeat: 'all', volume: 0.5, durationKnown: true,
    radio: { name: 'Night drive', keepsAdding: true, steer: 0.6, seed: { kind: 'artist' } } };
  p.ctx.App = {
    player: { current: () => track, playingCopy: () => ({ where: 'soulseek', name: 'mira', library: null, saved: false, received: 0.5, format: 'FLAC 16/44.1',
      peer: { name: 'mira', sizeMB: 32.8, speed: 1.2, rank: 1, of: 3 } }), playingFrom: () => ({ kind: 'album', id: 'al' }), duration: () => 225, state,
      upNext: () => [tracks.a, tracks.b, tracks.c], stateWords: () => 'Streaming from mira' },
    data: { formats: {}, track: (id) => tracks[id], album: () => ({ id: 'al', title: 'Al', year: 1984, label: 'Alfa', trackIds: ['x', 't1'] }), artist: () => ({ id: 'ar', name: 'A', listeners: 9 }) },
    fmt: { formatLine: () => '', time: () => '', size: () => '', length: () => '' },
    placeName: () => 'Al', placePath: () => '/album/al', isLiked: () => false, isSaved: () => false, store: { get: () => false },
  };
  return p;
}
const LIVE_JSON = JSON.parse(readFileSync(join(ROOT, 'web', 'designs', 'live.json'), 'utf8'));

test('every pipe the model lists is one live.js has, and no other', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 0));
  assert.deepEqual([...p.Live.pipes].sort(), Object.keys(LIVE_JSON.pipes).sort());
});

test('the records live.js builds have exactly the fields the model lists for them', () => {
  const p = withApp(page(new Date(2026, 10, 8, 14, 12, 0)));
  const fieldsOf = (rec) => { const r = LIVE_JSON.records[rec]; return [...(r['...'] ? fieldsOf(r['...']) : []), ...Object.keys(r).filter((k) => k !== '...')]; };
  for (const root of ['copy', 'player', 'place', 'radio']) {
    assert.deepEqual(Object.keys(p.Live.records[root]()).sort(), fieldsOf(LIVE_JSON.roots[root].of).sort(), root);
  }
  // a queued song is its track and its group, as the model's `queued` has it
  assert.equal(p.Live.records.next().group, 'yours');
  assert.deepEqual(JSON.parse(JSON.stringify(p.Live.records.queue().map((s) => s.group))), ['yours', 'yours', 'place']);
});

test('any field of a root is a value by path, in every form, and pipes turn it into words', () => {
  const size = el({ 'data-bind': 'copy.sizeBytes | size' }), no = el({ 'data-bind': 'song.no | pad:2' }), q = el({ 'data-bind': 'queue.3.title | upper' });
  const day = el({ 'data-bind': 'clock.now | date:weekday-short | upper' }), play = el({ 'data-bind': 'player.playing | pick:pause,play' });
  const label = el({ 'data-bind': 'album.label' }), host = el({ 'data-live': '' });
  const p = withApp(page(new Date(2026, 10, 8, 14, 12, 0), [size, no, q, day, play, label, host]));
  p.Live.use(['copy.sizeBytes', 'song.no', 'queue.3.title', 'clock.now', 'player.playing', 'album.label', 'copy.where', 'song.tags']);
  assert.equal(size.textContent, '32.8 MB');
  assert.equal(no.textContent, '04');
  assert.equal(q.textContent, 'FOUR');
  assert.equal(day.textContent, new Intl.DateTimeFormat(undefined, { weekday: 'short' }).format(new Date(2026, 10, 8)).toUpperCase());
  assert.equal(play.textContent, 'pause');
  assert.equal(label.textContent, 'Alfa');
  assert.equal(host.style.props['--song-no'], '4');
  assert.equal(host.style.props['--copy-where'], '"soulseek"');
  assert.equal(host.style.props['--song-tags'], '1', 'a list as a custom property is how many things it has');
  assert.equal(p.root.attrs['data-copy-where'], 'soulseek');
  assert.equal(p.Live.text('queue.1.duration', [['time', '']]), '3:45');
  assert.equal(p.Live.text('song.tags', [['join', ' / ']]), 'city pop');
  assert.equal(p.Live.known('copy.peers'), false);
  assert.equal(p.Live.known('queue.0.title'), false);
});

test('the pipes say what their names say', () => {
  const p = page(new Date(2026, 10, 8, 14, 12, 0));
  const t = (v, ...pipes) => { p.Live.use(['player.volume']); p.Live.feed({ 'player.volume': v }); return p.Live.text('player.volume', pipes); };
  assert.equal(t(0.42, ['pct', '']), '42%');
  assert.equal(t(0, ['or', '-']), '-');
  assert.equal(t(0.5, ['fixed', '2']), '0.50');
  assert.equal(t(1, ['map', '1=FULL,0=OFF']), 'FULL');
  assert.equal(t(0.3, ['map', '1=FULL']), '0.3');
});
