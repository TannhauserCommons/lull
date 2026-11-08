// Layout belongs to the theme (ADR-0101, section 4): every layout choice lull's script makes is a layout switch, a custom
// property lull's CSS sets (`--layout-*` on `.app`, web/components.css) and web/layout.js reads. So no script in web/
// decides layout by the window or a breakpoint of its own:
// - no script reads the window's size (innerWidth, innerHeight, outerWidth, outerHeight, screen.width, visualViewport);
// - no script asks a media query about a width, a height, an aspect ratio or an orientation (matchMedia);
// - no script compares a measured size (a box's width or height, its client, offset or scroll size, a resize
//   observer's contentRect) with a number of 100 or more or a constant (SHORT): a breakpoint by another name;
// - no script names a size class to decide by (App.layout.at('wide'), [data-size] compared): it asks a switch instead.
// The few measurements that are not layout choices (keeping a tooltip inside the window, fitting Classic's windows to
// their room) are ALLOWED below, each with its reason, and an entry that no longer matches anything fails too, so the
// list stays exact. docs/themes.md, section 5, Layout switches, lists the switches; their table is checked against
// components.css here as well.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const WEB = join(ROOT, 'web');
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];

/* lull's own script: not a vendored file, not a generated one (a built-in design's package, the hook reference) */
function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') return [];
    return statSync(path).isDirectory() ? walk(path) : [relative(WEB, path).replace(/\\/g, '/')];
  });
}
const scripts = walk(WEB).filter((f) => f.endsWith('.js') && !f.endsWith('.min.js') && f !== 'designs/hooks.js'
  && !DESIGNS.some((d) => f === `designs/${d}.js`));

/* the measurements that decide no layout, by file and a piece of their line, each with why */
const ALLOWED = [
  { file: 'app.js', has: 'window.innerWidth - w - 12', why: 'a popover is kept inside the window, wherever its anchor is' },
  { file: 'app.js', has: 'window.innerHeight - 12', why: 'a popover opens above its anchor when it would run off the window\'s foot' },
  { file: 'app.js', has: 'var lastW = window.innerWidth', why: 'popovers close when the window really resizes, not when a phone\'s keyboard shows' },
  { file: 'app.js', has: 'if (window.innerWidth !== lastW)', why: 'the same: a change of width, compared with the last, never with a breakpoint' },
  { file: 'app.js', has: 'window.innerWidth - w - 8', why: 'a tooltip is kept inside the window' },
  { file: 'screens/now.js', has: 'bottom: innerHeight', why: 'dragging a queue row scrolls its list; the window is the box when the list has none' },
  { file: 'desktop/desktop.js', has: 'r.bottom + 6 + th > innerHeight', why: 'the mini player\'s tooltip opens above its key at the window\'s foot' },
  { file: 'desktop/desktop.js', has: 'innerWidth - tw - 4', why: 'the mini player\'s tooltip is kept inside its window' },
  { file: 'desktop/desktop.js', has: 'h !== innerHeight', why: 'the mini player\'s window is sized to what it draws' },
  { file: 'designs/loops.js', has: 'r.top >= win.innerHeight', why: 'a loop off the screen is not checked (Theme Studio\'s loop advice)' },
];

/* comments out (line numbers kept): they describe, they don't decide */
const code = (text) => text.replace(/\/\*[\s\S]*?\*\//g, (c) => c.replace(/[^\n]/g, ' ')).replace(/(^|[^:'"\\])\/\/.*$/gm, '$1');

/* a measured size, a comparison (not =>, ==, <<), and a number of 100 or more or a constant (SHORT, a breakpoint by name) */
const SIZE = String.raw`\b(?:clientWidth|clientHeight|offsetWidth|offsetHeight|scrollWidth|scrollHeight|getBoundingClientRect|contentRect|width|height)\b`;
const CMP = String.raw`(?<![=<>!])(?:<=?|>=?)(?![=>])`, BIG = String.raw`(?:\d{3,}(?:\.\d+)?|[A-Z][A-Z0-9_]{2,})\b`;
const RULES = [
  { what: 'reads the window\'s size', re: /\b(?:innerWidth|innerHeight|outerWidth|outerHeight|visualViewport)\b|\bscreen\.(?:width|height|availWidth|availHeight)\b/ },
  { what: 'asks a media query about the window\'s size or shape', re: /matchMedia\(\s*['"`][^'"`]*\b(?:width|height|aspect-ratio|orientation)\b/ },
  { what: 'compares a measured size with a breakpoint', re: new RegExp(`${SIZE}[^;,]*?${CMP}\\s*\\(?\\s*${BIG}|(?<![\\w.])${BIG}\\s*${CMP}[^;,]*?${SIZE}`) },
  { what: 'names a size class to decide by', re: /layout\.at\(\s*['"`]|getAttribute\(\s*['"]data-(?:size|height)['"]\s*\)\s*[!=]==?\s*['"]/ },
];

function findings() {
  const out = [];
  for (const file of scripts) {
    code(readFileSync(join(WEB, file), 'utf8')).split('\n').forEach((line, i) => {
      for (const rule of RULES) if (rule.re.test(line)) out.push({ file, line: i + 1, text: line.trim(), what: rule.what });
    });
  }
  return out;
}

test('no script in web/ decides layout by the window or a breakpoint: it reads a layout switch', () => {
  const found = findings();
  const allowed = (f) => ALLOWED.some((a) => a.file === f.file && f.text.includes(a.has));
  const bad = found.filter((f) => !allowed(f)).map((f) => `web/${f.file}:${f.line} ${f.what}: ${f.text}`);
  assert.deepEqual(bad, [], 'a layout choice in script: make it a layout switch (a --layout-* property lull\'s CSS sets on .app, read in web/layout.js; '
    + 'docs/themes.md, section 5, Layout switches), or, for a measurement that decides no layout, add it to ALLOWED with why');
  const stale = ALLOWED.filter((a) => !found.some((f) => f.file === a.file && f.text.includes(a.has))).map((a) => `${a.file}: ${a.has}`);
  assert.deepEqual(stale, [], 'ALLOWED names a measurement that is gone: take it out');
});

test('the check finds what it is for', () => {
  const hits = (line) => RULES.filter((r) => r.re.test(line)).map((r) => r.what);
  for (const line of [
    "if (window.innerWidth >= 1360) dock();",
    "var phone = matchMedia('(max-width: 639px)').matches;",
    "if (el.getBoundingClientRect().width < 640) phone();",
    "short = e.contentRect.height < 760;",
    "else short = e.contentRect.height < SHORT;",
    "if (1100 <= main.clientWidth) wide();",
    "if (App.layout.at('wide')) split();",
  ]) assert.notDeepEqual(hits(line), [], line);
  for (const line of [
    "return s.scrollWidth <= s.clientWidth + 1;",
    "if (e.scrollWidth > e.clientWidth) return true;",
    "w = clamp(w0 - (ev[along] - p0), LIMITS[d][0], LIMITS[d][1]);",
    "if (w >= parseFloat(S[FROM[CLASSES[i]]])) return CLASSES[i];",
    "matchMedia('(prefers-reduced-motion: reduce)').matches",
    "setTimeout(done, 1100); var h = el.offsetHeight;",
  ]) assert.deepEqual(hits(line), [], line);
});

/* the switches lull's CSS registers are the ones layout.js reads and the docs' table lists, with the same values */
test('every layout switch is registered in components.css, read by layout.js and in the docs\' table', () => {
  const css = readFileSync(join(WEB, 'components.css'), 'utf8');
  const registered = new Map([...css.matchAll(/@property (--(?:layout-[\w-]+|fit-order|fit-sides)) \{ syntax: "([^"]+)"/g)].map((m) => [m[1], m[2]]));
  const layoutJs = readFileSync(join(WEB, 'layout.js'), 'utf8');
  const read = new Set([...layoutJs.matchAll(/'(--layout-[\w-]+|--fit-order|--fit-sides)'/g)].map((m) => m[1]));   // named in its code
  const docs = readFileSync(join(ROOT, 'docs', 'themes.md'), 'utf8');
  const from = docs.indexOf('**Layout switches'), to = docs.indexOf('\n\n**', from + 1);
  assert.ok(from > 0, 'docs/themes.md, section 5, has the Layout switches table');
  const table = docs.slice(from, to);
  for (const [name, syntax] of registered) {
    assert.ok(read.has(name), `web/layout.js reads ${name}`);
    const row = table.split('\n').find((l) => l.split('|')[1]?.includes(`\`${name}\``));
    assert.ok(row, `docs/themes.md's Layout switches table lists ${name}`);
    for (const v of syntax.split('|').map((s) => s.trim()).filter((s) => !s.startsWith('<'))) assert.ok(row.includes(`\`${v}\``), `the docs' row for ${name} names its value ${v}`);
  }
  for (const name of read) assert.ok(registered.has(name), `${name}, which web/layout.js reads, is registered in components.css`);
});
