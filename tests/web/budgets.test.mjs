// The shipped-bytes check (scripts/budgets, docs/context/quality.md, Budgets check): how a listed file is sized, which
// requests are files of the build, what a route key must be, and when growth fails. The listing itself is a real browser
// and runs in `mise run budgets`. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { sizeOf, wireFile } from '../../scripts/budgets/measure.mjs';
import { assertScreen, builtFile } from '../../scripts/budgets/list.mjs';
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

const limits = {
  allowance: { bytes: 500, why: 'an ordinary change' },
  routes: {
    home: { ceiling: { bytes: 10_000, why: 'what home ships now' } },
    now: { ceiling: { bytes: 10_000, why: 'x' }, allowance: { bytes: 100, why: 'now barely moves' } },
  },
};
const side = (home, now, extra = {}) => ({ home: { bytes: home, files: [{ path: 'app.js', bytes: home }] }, now: { bytes: now, files: [{ path: 'app.js', bytes: now }] }, ...extra });
const verdict = (base, change) => decide({ base, change, limits }).filter((r) => r.problems.length).map((r) => [r.route, r.problems.map((p) => p.kind)]);

test('growth within the allowance passes, past it fails, and shrinking never does', () => {
  assert.deepEqual(verdict(side(5000, 5000), side(5500, 5100)), []);
  assert.deepEqual(verdict(side(5000, 5000), side(5501, 5000)), [['home', ['growth']]]);
  assert.deepEqual(verdict(side(5000, 5000), side(5000, 5101)), [['now', ['growth']]]);
  assert.deepEqual(verdict(side(9000, 5000), side(4000, 5000)), []);
});

test('the ceiling holds however small the growth, and a route the base lacks is judged by it alone', () => {
  assert.deepEqual(verdict(side(9900, 5000), side(10_001, 5000)), [['home', ['ceiling']]]);
  assert.deepEqual(verdict(side(9900, 5000), side(10_450, 5000)), [['home', ['growth', 'ceiling']]]);
  assert.deepEqual(verdict({ now: side(5000, 5000).now }, side(5000, 5000)), []);
});

test('a measured route no limits know fails loudly, and so does a limit without a reason or a whole number', () => {
  assert.throws(() => decide({ base: null, change: side(1, 1, { search: { bytes: 1, files: [] } }), limits }), /search is measured but has no limits/);
  assert.throws(() => validateLimits({ ...limits, allowance: { bytes: 500, why: ' ' } }), /reason/);
  assert.throws(() => validateLimits({ ...limits, routes: { home: { ceiling: { bytes: 1.5, why: 'x' } } } }), /whole number/);
});

test('a new route goes green in the change that adds it, by its own ceiling; a route the change drops fails', () => {
  const own = { ...limits, routes: { ...limits.routes, search: { ceiling: { bytes: 6000, why: 'search ships what home does' } } } };
  const search = (bytes) => ({ search: { bytes, files: [{ path: 'app.js', bytes }] } });
  const rows = (change, base = side(5000, 5000)) => decide({ base, change: { ...side(5000, 5000), ...change }, limits, ownLimits: own });
  const added = rows(search(5900)).find((r) => r.route === 'search');
  assert.deepEqual([added.isNew, added.base, added.growth, added.problems], [true, null, 0, []]);
  assert.deepEqual(rows(search(6001)).find((r) => r.route === 'search').problems.map((p) => p.kind), ['ceiling']);
  // the merge-base's own entry judges a route it knows, whatever the change's file says of it
  const loosened = { ...own, routes: { ...own.routes, home: { ceiling: { bytes: 99_999, why: 'loosened' } } } };
  const home = decide({ base: side(5000, 5000), change: side(10_001, 5000), limits, ownLimits: loosened }).find((r) => r.route === 'home');
  assert.deepEqual(home.problems.map((p) => p.kind), ['growth', 'ceiling']);
  const dropped = decide({ base: side(5000, 5000), change: { home: side(5000, 5000).home }, limits });
  assert.deepEqual(dropped.map((r) => [r.route, !!r.removed, r.problems.map((p) => p.kind)]), [['home', false, []], ['now', true, ['removed']]]);
  const verdict = report(dropped, 'f');
  assert.equal(verdict.ok, false);
  assert.match(verdict.lines.join('\n'), /now .*REMOVED/);
  assert.match(verdict.lines.join('\n'), /now: budgeted by the merge-base, and no longer in this change's budgets file/);
  assert.match(verdict.lines.join('\n'), /leaves the budgets only by the owner's own commit/);
  const { ok, lines } = report(rows(search(5900)), 'f');
  assert.equal(ok, true);
  assert.match(lines.join('\n'), /search +new +5,900 .* ok \(new\)/);
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
  assert.equal(report(decide({ base, change: base, limits }), 'f').ok, true);
  // Routes that grew the same way are one entry in the message.
  const both = { home: change.home, now: { ...change.home } };
  assert.match(report(decide({ base: { home: base.home, now: base.home }, change: both, limits: { ...limits, routes: { home: limits.routes.home, now: { ceiling: limits.routes.home.ceiling } } } }), 'f').lines.join('\n'), /budgets: home, now: grew 1,000 B/);
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
