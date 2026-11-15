// The shipped-bytes check (scripts/budgets, docs/context/quality.md, Check in shadow): which files a route's start counts and
// at what size, and when growth over the merge-base fails. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { cssRefs, indexFiles, startOf } from '../../scripts/budgets/measure.mjs';
import { changedFiles, decide, report, validateLimits } from '../../scripts/budgets/decide.mjs';

const INDEX = `<!doctype html><html><head>
<link rel="icon" href="assets/app-icon.svg"><link rel="manifest" href="manifest.webmanifest"><link rel="apple-touch-icon" href="a.png">
<link rel=stylesheet href=tokens.css><link rel="stylesheet" id="design-css" blocking="render">
<style>@import url(components.css) layer(components);</style>
<script>if (window.__TAURI__) document.write('<style>@import url(screens/classic.css)<\\/style><script src="desktop/desktop.js"><\\/script>');</script>
<script src="designs/designs.js"></script></head><body><script src="app.js"></script></body></html>`;

// A build as web-dist writes it: a number is a file of that many bytes, a string is a file with that text. The .br and
// .gz beside a file are what the hub sends when it has them.
function build(spec) {
  const dir = mkdtempSync(join(tmpdir(), 'budgets-test-'));
  for (const [path, content] of Object.entries(spec)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), typeof content === 'string' ? content : Buffer.alloc(content, 1));
  }
  return dir;
}
const files = (over = {}) => ({
  'index.html': INDEX, 'index.html.br': 100,
  'tokens.css': '@font-face{src:url(fonts/nunito.woff2)}', 'tokens.css.br': 50,
  'components.css': '.a{}', 'components.css.gz': 90, 'components.css.br': 80,
  'designs/designs.js': 'x.fallback="lull"', 'designs/designs.js.br': 30,
  'app.js': 200, 'app.js.gz': 70,
  'designs/lull.js': 40, 'designs/lull.js.br': 20,
  'designs/lull/theme.css': '@font-face{src:url(assets/audiowide.woff2)}.d{background:url(assets/stars.svg)}', 'designs/lull/theme.css.br': 25,
  'fonts/nunito.woff2': 1000, 'designs/lull/assets/audiowide.woff2': 500, 'designs/lull/assets/stars.svg': 777,
  'screens/settings-fit.js': 90, 'screens/settings-fit.js.br': 9,
  ...over,
});
const model = {
  notAtStart: { 'designs/lull/assets/stars.svg': 'from 1,100 px' },
  routes: { home: {}, 'settings/playback': { loads: { 'screens/settings-fit.js': 'its fits' } } },
};

test('index.html: its stylesheets, imports and scripts, never the inline script\'s strings, never what loads after the page', () => {
  assert.deepEqual(indexFiles(INDEX), {
    stylesheets: ['tokens.css', 'components.css'],
    scripts: ['designs/designs.js', 'app.js'],
  });
});

test('index.html: a link or an element that would fetch something unmeasured is refused, not skipped', () => {
  assert.throws(() => indexFiles('<link rel="preload" href="x.woff2">'), /rel="preload"/);
  assert.throws(() => indexFiles('<body><img src="x.png"></body>'), /fetches a file/);
});

test('a stylesheet: imports and @font-face sources apart from every other url(), data: left alone', () => {
  const css = '@import url(a.css) layer(x);@font-face{font-family:F;src:url("f.woff2") format("woff2")}'
    + '.a{background:url(img/b.svg)}.c{background:url(data:image/png;base64,AAA)}@import "d.css";';
  assert.deepEqual(cssRefs(css), { imports: ['a.css', 'd.css'], fonts: ['f.woff2'], others: ['img/b.svg'] });
});

test('a route\'s start: brotli, else gzip, else the file as is, each file once, fonts and loads counted, listed-away urls not', () => {
  const dir = build(files());
  try {
    const home = startOf(dir, 'home', model);
    const bytes = Object.fromEntries(home.files.map((f) => [f.path, f.bytes]));
    assert.deepEqual(bytes, {
      'app.js': 70, 'components.css': 80, 'designs/designs.js': 30, 'designs/lull.js': 20, 'designs/lull/assets/audiowide.woff2': 500,
      'designs/lull/theme.css': 25, 'fonts/nunito.woff2': 1000, 'index.html': 100, 'tokens.css': 50,
    });
    assert.equal(home.bytes, 1875);
    assert.deepEqual([...home.skipped], ['designs/lull/assets/stars.svg']);
    // The route's own code is its own bytes on top, as the hub sends it.
    assert.equal(startOf(dir, 'settings/playback', model).bytes, 1875 + 9);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('a start stylesheet with a url() nobody has decided on fails the measurement', () => {
  const dir = build(files({ 'tokens.css': '.a{background:url(new.png)}' }));
  try {
    assert.throws(() => startOf(dir, 'home', model), /tokens\.css loads new\.png.*notAtStart/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
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

test('a measured route without limits, or limits without a measurement, fail loudly; so does a limit without a reason', () => {
  assert.throws(() => decide({ base: null, change: side(1, 1, { search: { bytes: 1, files: [] } }), limits }), /search is measured but has no limits/);
  assert.throws(() => decide({ base: null, change: { home: side(1, 1).home }, limits }), /now has limits.*not measured/);
  assert.throws(() => validateLimits({ ...limits, allowance: { bytes: 500, why: ' ' } }), /reason/);
  assert.throws(() => validateLimits({ ...limits, routes: { home: { ceiling: { bytes: 1.5, why: 'x' } } } }), /whole number/);
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
