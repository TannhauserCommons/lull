// The shipped-bytes check (scripts/budgets, docs/context/quality.md, Budgets check): how a listed file is sized, which
// requests are files of the build, what a route key must be, and when growth fails. The listing itself is a real browser
// and runs in `mise run budgets`. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { sizeOf, wireFile } from '../../scripts/budgets/measure.mjs';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { assertScreen, BLOCKED_PORTS, builtFile, classicDelay, horizonOf, listening, MIN_HORIZON_MS } from '../../scripts/budgets/list.mjs';
import { changedFiles, decide, report, validateLimits } from '../../scripts/budgets/decide.mjs';
import { checked, chooseHarness } from '../../scripts/budgets/run.mjs';

// A build as web-dist writes it: a number is a file of that many bytes. The .br and .gz beside a file are what the hub sends.
function build(spec) {
  const dir = mkdtempSync(join(tmpdir(), 'budgets-test-'));
  for (const [path, size] of Object.entries(spec)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), Buffer.alloc(size, 1));
  }
  return dir;
}
const within = (spec, fn) => {
  const dir = build(spec);
  try { return fn(dir); } finally { rmSync(dir, { recursive: true, force: true }); }
};

test('a listed file counts at what the hub sends: brotli, else gzip, else the file as it is', () => {
  within({ 'app.js': 200, 'app.js.gz': 70, 'app.js.br': 60, 'base.css': 90, 'base.css.gz': 40, 'fonts/n.woff2': 1000 }, (dir) => {
    assert.deepEqual(wireFile(dir, 'app.js'), { path: 'app.js', encoding: 'br', bytes: 60 });
    assert.deepEqual(wireFile(dir, 'base.css'), { path: 'base.css', encoding: 'gz', bytes: 40 });
    assert.deepEqual(wireFile(dir, 'fonts/n.woff2'), { path: 'fonts/n.woff2', encoding: 'identity', bytes: 1000 });
    const sized = sizeOf(dir, new Set(['fonts/n.woff2', 'app.js', 'base.css']));
    assert.deepEqual(sized.files.map((f) => f.path), ['app.js', 'base.css', 'fonts/n.woff2']);
    assert.equal(sized.bytes, 1100);
    assert.throws(() => wireFile(dir, 'gone.js'), /gone\.js was fetched but the build made no such file/);
  });
});

test('a request is a file of the build when it is the page\'s own origin and a file in the tree, whatever the query', () => {
  within({ 'index.html': 5, 'designs/lull/theme.css': 5, 'api/not-a-hub-file.json': 5 }, (dir) => {
    const origin = 'http://127.0.0.1:8790';
    const of = (path) => builtFile(origin + path, origin, dir);
    assert.equal(of('/'), 'index.html');
    assert.equal(of('/designs/lull/theme.css?v=3'), 'designs/lull/theme.css');
    assert.equal(of('/designs/lull/%74heme.css'), 'designs/lull/theme.css');
    assert.equal(of('/designs/lull/'), null);
    assert.equal(of('/designs/none.css'), null);
    assert.equal(of('/../../etc/passwd'), null);
    assert.equal(builtFile('http://elsewhere.example/designs/lull/theme.css', origin, dir), null);
  });
});

test('a route key that matches no screen, or redirects, fails the listing instead of listing the shell', () => {
  assert.doesNotThrow(() => assertScreen('library/albums', { routed: true, path: '/library/albums' }));
  assert.throws(() => assertScreen('hmoe', { routed: false, path: '/hmoe' }), /"hmoe" is not a screen.*matched no screen/);
  assert.throws(() => assertScreen('library', { routed: true, path: '/library/albums' }), /"library" is not a screen.*redirected to \/library\/albums/);
});

const appJs = readFileSync(fileURLToPath(new URL('../../web/app.js', import.meta.url)), 'utf8');
const deferred = (ms) => `x();\n  App.on('ready', function () {\n    setTimeout(function () {\n      var go = function () { App.needClassic().then(null, function () {}); };\n      if (window.requestIdleCallback) requestIdleCallback(go, { timeout: 10000 }); else go();\n    }, ${ms});\n  });\ny();`;

test('the start ends a second short of Classic\'s deferred load, read from app.js, and a delay too short for a start fails', () => {
  assert.equal(classicDelay(deferred(15000)), 15000);
  assert.equal(classicDelay(appJs), 15000);
  assert.equal(horizonOf(deferred(15000)), 14000);
  assert.equal(horizonOf(deferred(MIN_HORIZON_MS + 1000)), MIN_HORIZON_MS);
  assert.throws(() => horizonOf(deferred(MIN_HORIZON_MS + 999)), /leaves a start of 9999 ms/);
  assert.throws(() => classicDelay("App.on('ready', function () {\n    other();\n  });"), /no longer loads Classic/);
  assert.throws(() => classicDelay('nothing here'), /no longer loads Classic/);
});

test('the fixture hub never gets a port a browser refuses', () => {
  for (const port of [6665, 6666, 6000, 6697, 10080, 4045]) assert.ok(BLOCKED_PORTS.has(port), String(port));
  for (const port of [8790, 8080, 5030, 49152]) assert.ok(!BLOCKED_PORTS.has(port), String(port));
});

const fakeHub = () => Object.assign(new EventEmitter(), { stdout: new EventEmitter(), stderr: new EventEmitter() });

test('the fixture hub is ours only when it says so; a port taken in the gap says so, and anything else that stops it is an error', async () => {
  const up = fakeHub();
  const result = listening(up, 7001);
  up.stdout.emit('data', 'fixture hub on http://127.0.0.1:7001\n');
  assert.equal(await result, 'up');
  const taken = fakeHub();
  const retry = listening(taken, 7002);
  taken.stderr.emit('data', 'Error: listen EADDRINUSE: address already in use 127.0.0.1:7002');
  taken.emit('exit', 1);
  assert.equal(await retry, 'in-use');
  const crashed = fakeHub();
  const failed = listening(crashed, 7003);
  crashed.stderr.emit('data', 'SyntaxError: boom');
  crashed.emit('exit', 1);
  await assert.rejects(failed, /stopped before it was listening on 7003: SyntaxError: boom/);
});

const limits = {
  allowance: { bytes: 500, why: 'an ordinary change' },
  routes: {
    home: {},
    now: { allowance: { bytes: 100, why: 'now barely moves' } },
  },
};
const side = (home, now, extra = {}) => ({ home: { bytes: home, files: [{ path: 'app.js', bytes: home }] }, now: { bytes: now, files: [{ path: 'app.js', bytes: now }] }, ...extra });
const route = (bytes) => ({ bytes, files: [{ path: 'app.js', bytes }] });
const kinds = (args) => decide({ limits, ...args }).rows.filter((r) => r.problems.length).map((r) => [r.route, r.problems.map((p) => p.kind)]);
const verdict = (base, change) => kinds({ base, change });
const withRoutes = (routes, allowance = limits.allowance) => ({ allowance, routes });

test('growth within the allowance passes, past it fails, and shrinking never does', () => {
  assert.deepEqual(verdict(side(5000, 5000), side(5500, 5100)), []);
  assert.deepEqual(verdict(side(5000, 5000), side(5501, 5000)), [['home', ['growth']]]);
  assert.deepEqual(verdict(side(5000, 5000), side(5000, 5101)), [['now', ['growth']]]);
  assert.deepEqual(verdict(side(9000, 5000), side(4000, 5000)), []);
});

test('a change that grows a start by more than 500 B fails with the files that grew; 500 B exactly passes', () => {
  const home = (bytes, extra = []) => ({ bytes, files: [{ path: 'app.js', bytes: 5000 }, ...extra] });
  const base = { home: home(5000) };
  const within = report(decide({ base, change: { home: home(5500, [{ path: 'screens/a.js', bytes: 500 }]) }, limits: withRoutes({ home: {} }) }), 'f');
  assert.equal(within.ok, true);
  const over = report(decide({ base, change: { home: home(5501, [{ path: 'screens/a.js', bytes: 501 }]) }, limits: withRoutes({ home: {} }) }), 'f');
  assert.equal(over.ok, false);
  assert.match(over.lines.join('\n'), /home: grew 501 B over the merge-base \(5,000 B to 5,501 B\); the allowance is 500 B/);
  assert.match(over.lines.join('\n'), /what grew: screens\/a\.js \+501 B \(new at start\)/);
});

test('there is no fixed ceiling: a route is judged by its growth over the merge-base, however large it already is', () => {
  assert.deepEqual(verdict(side(900_000, 5000), side(900_400, 5000)), []);
  assert.deepEqual(verdict(side(900_000, 5000), side(900_501, 5000)), [['home', ['growth']]]);
  assert.ok(decide({ base: side(1, 1), change: side(2, 1), limits }).rows.every((r) => !('ceiling' in r)));
});

test('every allowance is a whole number of bytes with a reason; 0 allows no growth and is never "off"', () => {
  assert.throws(() => decide({ base: null, change: side(1, 1, { search: route(1) }), limits }), /search is measured but has no limits/);
  assert.throws(() => validateLimits({ ...limits, allowance: { bytes: 500, why: ' ' } }), /reason/);
  assert.throws(() => validateLimits({ ...limits, routes: { home: 'ceiling' } }), /home: a route is an object/);
  assert.throws(() => validateLimits({ ...limits, routes: { home: { ceiling: { bytes: 10_000, why: 'x' } } } }), /home: unknown limit "ceiling"/);
  for (const allowance of [0, false, null, '', NaN, { bytes: NaN, why: 'x' }, { bytes: -1, why: 'x' }, { bytes: '500', why: 'x' }, { bytes: 1.5, why: 'x' }]) {
    assert.throws(() => validateLimits({ ...limits, routes: { home: { allowance } } }), /home allowance/, String(allowance));
    assert.throws(() => validateLimits({ ...limits, allowance }), /allowance/, String(allowance));
  }
  const none = withRoutes({ home: {} }, { bytes: 0, why: 'no growth at all' });
  const grow = (by) => decide({ base: { home: route(5000) }, change: { home: route(5000 + by) }, limits: none }).rows[0].problems.map((p) => p.kind);
  assert.deepEqual([grow(0), grow(1)], [[], ['growth']]);
  assert.throws(() => decide({ base: side(1, 1), change: side(900_000, 1), limits: { ...limits, allowance: { bytes: NaN, why: 'x' } } }), /whole number/);
});

test('a route new in the change has nothing to grow from: it is named by its own entry and printed as new', () => {
  const own = withRoutes({ ...limits.routes, search: {} });
  const result = decide({ base: side(5000, 5000), change: { ...side(5000, 5000), search: route(900_000) }, limits, ownLimits: own });
  const added = result.rows.find((r) => r.route === 'search');
  assert.deepEqual([added.isNew, added.base, added.growth, added.problems], [true, null, 0, []]);
  const { ok, lines } = report(result, 'f');
  assert.equal(ok, true);
  assert.match(lines.join('\n'), /search +new +900,000 .* ok \(new\)/);
});

test('the merge-base\'s allowance judges a route it knows: a change that grew cannot loosen it', () => {
  const loosened = withRoutes({ ...limits.routes }, { bytes: 99_999, why: 'loosened' });
  assert.deepEqual(kinds({ base: side(9000, 5000), change: side(9600, 5000), ownLimits: loosened }), [['home', ['growth']]]);
});

test('the owner\'s own limit changes land when nothing grew: the allowance changed, a route removed or renamed', () => {
  const base = side(9900, 5000);
  // the allowance changed in a change that grew nothing: the change's own file judges, and says so
  const changed = decide({ base, change: base, limits, ownLimits: withRoutes(limits.routes, { bytes: 800, why: 'the owner raised it' }) });
  assert.deepEqual([changed.rows.filter((r) => r.problems.length).length, changed.notes], [0, ["limits changed, nothing grew: this change's own budgets file judges it"]]);
  assert.match(report(changed, 'f').lines.join('\n'), /limits changed, nothing grew/);
  // a route removed together with its screen
  const gone = decide({ base, change: { home: base.home, now: { gone: true } }, limits, ownLimits: withRoutes({ home: limits.routes.home }) });
  assert.deepEqual(gone.rows.map((r) => [r.route, !!r.removed, !!r.gone, r.problems.length]), [['home', false, false, 0], ['now', true, true, 0]]);
  assert.match(report(gone, 'f').lines.join('\n'), /now .*removed \(screen gone\)/);
  // a route renamed with its screen: the old key's screen is gone, the new key is new
  const renamed = decide({ base, change: { home: base.home, now: { gone: true }, 'now-playing': route(5000) }, limits,
    ownLimits: withRoutes({ home: limits.routes.home, 'now-playing': {} }) });
  assert.deepEqual(renamed.rows.map((r) => [r.route, !!r.isNew, !!r.gone, r.problems.length]), [['home', false, false, 0], ['now', false, true, 0], ['now-playing', true, false, 0]]);
  assert.equal(report(renamed, 'f').ok, true);
});

test('abuse: a route removed while its screen exists, or while it grew, or renamed with the screen still there, fails', () => {
  const base = side(5000, 5000);
  const only = (routes) => withRoutes(routes);
  // removed from the budgets, the screen still answers (and nothing grew): a failure, not a quiet drop
  const exists = decide({ base, change: base, limits, ownLimits: only({ home: limits.routes.home }) });
  assert.deepEqual(exists.rows.filter((r) => r.problems.length).map((r) => [r.route, r.problems.map((p) => p.kind)]), [['now', ['removed']]]);
  assert.equal(report(exists, 'f').ok, false);
  assert.match(report(exists, 'f').lines.join('\n'), /now .*REMOVED[\s\S]*its screen still exists/);
  // removed while it grew: the base's limits judge, and it fails
  const grew = decide({ base, change: side(5000, 5800), limits, ownLimits: only({ home: limits.routes.home }) });
  assert.deepEqual(grew.notes, []);
  assert.deepEqual(grew.rows.filter((r) => r.problems.length).map((r) => [r.route, r.problems.map((p) => p.kind)]), [['now', ['removed']]]);
  // the same key renamed while the screen still exists: the old key is still a screen
  const renamed = decide({ base, change: { home: base.home, now: base.now, 'now-x': route(5000) }, limits, ownLimits: only({ home: limits.routes.home, 'now-x': {} }) });
  assert.deepEqual(renamed.rows.filter((r) => r.problems.length).map((r) => r.route), ['now']);
  // growth elsewhere with the allowance raised in the same change: the base's limits judge
  assert.deepEqual(kinds({ base, change: side(5600, 5000), ownLimits: withRoutes({ home: {}, now: limits.routes.now }, { bytes: 99_999, why: 'loosened' }) }), [['home', ['growth']]]);
});

test('the report says what grew, by how much, against which allowance, and what to do', () => {
  const base = { ...side(5000, 5000), home: { bytes: 5000, files: [{ path: 'app.js', bytes: 5000 }] } };
  const change = { ...side(6000, 5000), home: { bytes: 6000, files: [{ path: 'app.js', bytes: 5200 }, { path: 'screens/new.js', bytes: 800 }] } };
  const { ok, lines } = report(decide({ base, change, limits }), 'scripts/budgets/shipped-bytes.json');
  const text = lines.join('\n');
  assert.equal(ok, false);
  assert.match(text, /home: grew 1,000 B over the merge-base \(5,000 B to 6,000 B\); the allowance is 500 B/);
  assert.match(text, /what grew: screens\/new\.js \+800 B \(new at start\), app\.js \+200 B/);
  assert.match(text, /impact note/);
  assert.match(text, /never passes/);
  assert.match(text, /scripts\/budgets\/shipped-bytes\.json/);
  assert.doesNotMatch(text, /ceiling/);
  assert.equal(report(decide({ base, change: base, limits }), 'f').ok, true);
  // Routes that grew the same way are one entry in the message.
  const both = { home: change.home, now: { ...change.home } };
  const twins = { ...limits, routes: { home: {}, now: {} } };
  assert.match(report(decide({ base: { home: base.home, now: base.home }, change: both, limits: twins }), 'f').lines.join('\n'), /budgets: home, now: grew 1,000 B/);
});


test('changedFiles lists additions, growth and removals, biggest growth first', () => {
  const before = { files: [{ path: 'a.js', bytes: 10 }, { path: 'gone.js', bytes: 30 }, { path: 'same.js', bytes: 5 }] };
  const after = { files: [{ path: 'a.js', bytes: 15 }, { path: 'new.js', bytes: 100 }, { path: 'same.js', bytes: 5 }] };
  assert.deepEqual(changedFiles(before, after).map((f) => [f.path, f.delta]), [['new.js', 100], ['a.js', 5], ['gone.js', -30]]);
});

test('the merge-base\'s harness judges; a change may judge itself only when it adds the harness', () => {
  assert.equal(chooseHarness({ baseHasHarness: true, addedByChange: false, rev: 'a'.repeat(40) }), 'base');
  assert.equal(chooseHarness({ baseHasHarness: false, addedByChange: true, rev: 'a'.repeat(40) }), 'own');
  assert.throws(() => chooseHarness({ baseHasHarness: false, addedByChange: false, rev: 'abcdef0123456789' }), /abcdef012345 has no budgets harness.*does not add it/);
});

test('a child process that did not start, was killed or failed is an error, not a status of null', () => {
  assert.equal(checked({ status: 0 }, 'x').status, 0);
  assert.throws(() => checked({ status: null, error: new Error('spawn git ENOENT') }, 'git archive'), /git archive: spawn git ENOENT/);
  assert.throws(() => checked({ status: null, signal: 'SIGKILL' }, 'tar'), /tar was ended by SIGKILL/);
  assert.throws(() => checked({ status: null, error: Object.assign(new Error('spawnSync git ENOBUFS'), { code: 'ENOBUFS' }) }, 'git archive'), /ENOBUFS/);
  assert.throws(() => checked({ status: 128, stderr: Buffer.from('fatal: bad revision') }, 'git archive'), /failed \(128\): fatal: bad revision/);
  assert.throws(() => checked(spawnSync('lull-no-such-program-7f3a', []), 'it'), /^Error: it: /);
});
