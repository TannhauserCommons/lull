// docs/themes.md is the theme API (ADR-0004). Its hook reference (section 11) names every class and data-* attribute
// lull's CSS styles or its markup sets, each either stable or internal, so the docs can't fall behind web/:
// - every class block in web/'s own CSS, and every hook a built-in design styles, is listed in one of the two;
// - every data-* attribute web/ sets or styles is listed in one of the two;
// - every hook a built-in design names exists in web/ (a rule for a hook that is gone never matches);
// - every name the reference lists still exists in web/.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const WEB = join(ROOT, 'web');
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
const OWN = /^(lull|sd|sl|pt|gh)-/;              // a built-in design's own classes (docs/themes.md, versioning)

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') return [];
    return statSync(path).isDirectory() ? walk(path) : [relative(WEB, path).replace(/\\/g, '/')];
  });
}
const files = walk(WEB);
const read = (f) => readFileSync(join(WEB, f), 'utf8');
const designCss = (f) => DESIGNS.some((d) => f === `designs/${d}/theme.css`);
const generated = (f) => DESIGNS.some((d) => f === `designs/${d}.js`);    // theme.json + slots, written by lull-themes
const hooksJs = (f) => f === 'designs/hooks.js';                            // this reference, written by the test below

const sharedCss = files.filter((f) => f.endsWith('.css') && !designCss(f));
const markup = files.filter((f) => /\.(js|html)$/.test(f) && !generated(f) && !hooksJs(f) && !f.endsWith('.min.js'));

/* the selectors of a stylesheet (comments and strings dropped, declarations skipped) */
function selectors(css) {
  css = css.replace(/\/\*[\s\S]*?\*\//g, '').replace(/"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'/g, '""');
  const out = [];
  let seg = '';
  for (const ch of css) {
    if (ch === '{') { const s = seg.slice(seg.lastIndexOf(';') + 1).trim(); if (!s.startsWith('@')) out.push(s); seg = ''; }
    else if (ch === '}') seg = '';
    else seg += ch;
  }
  return out.join('\n');
}
const classesOf = (sel) => new Set((sel.match(/\.[a-zA-Z_][\w-]*/g) || []).map((c) => c.slice(1)));
const attrsOf = (text) => new Set(text.match(/data-[a-z][a-z0-9]*(?:-[a-z0-9]+)*/g) || []);
/* a BEM block: .trow__num and .trow--dense are .trow's; state classes (.is-*, .has-*) stand alone */
const block = (c) => (/^(is|has)-/.test(c) ? c : c.replace(/(__|--).*$/, ''));

const shared = selectors(sharedCss.map(read).join('\n'));
const designs = DESIGNS.map((d) => ({ name: d, sel: selectors(read(`designs/${d}/theme.css`)) }));
const source = markup.map(read).join('\n') + '\n' + shared;

/* names markup builds from a prefix (`now-ly--${key}`, 'c' + n): the prefix stands for all of them */
const dynamic = [...source.matchAll(/([a-zA-Z][\w-]*?(?:-|__))(?:\$\{|['"]\s*\+)/g)].map((m) => m[1]);
const literal = (name) => new RegExp(`(?<![\\w-])${name.replace(/-/g, '\\-')}(?![\\w-])`).test(source);
const exists = (name) => literal(name) || dynamic.some((p) => name.startsWith(p) && name.length > p.length);

/* the reference: section 11 of docs/themes.md; "### Internal" and below is internal, the rest stable */
const docs = readFileSync(join(ROOT, 'docs/themes.md'), 'utf8');
const from = docs.indexOf('\n## 11. ');
const next = docs.indexOf('\n## ', from + 1);
const ref = from < 0 ? '' : docs.slice(from, next < 0 ? undefined : next);
const cut = ref.indexOf('\n### Internal');
function names(md) {
  const spans = (md.match(/`[^`\n]+`/g) || []).join(' ');
  return {
    classes: new Set((spans.match(/(?<![\w-])\.[a-zA-Z_][\w-]*\*?/g) || []).map((c) => c.slice(1))),
    attrs: new Set(spans.match(/data-[a-z][a-z0-9-]*\*?/g) || []),
  };
}
const stable = names(ref.slice(0, cut));
const internal = names(ref.slice(cut));
const listed = (set, name) => set.has(name) || [...set].some((n) => n.endsWith('*') && name.startsWith(n.slice(0, -1)));
const known = (kind, name) => listed(stable[kind], name) || listed(internal[kind], name);
const missing = (label, list) => {
  const uniq = [...new Set(list)].sort();
  return uniq.length ? `${label} (${uniq.length}): ${uniq.join(' ')}` : '';
};

test('the hook reference is in docs/themes.md', () => {
  assert.ok(ref.length > 1000 && cut > 0, 'docs/themes.md needs "## 11. Hook reference" with an "### Internal" part');
});

test('every class lull styles is listed as stable or internal', () => {
  const blocks = [...classesOf(shared)].map(block)
    .concat(designs.flatMap((d) => [...classesOf(d.sel)].filter((c) => !OWN.test(c)).map(block)));
  const bad = blocks.filter((b) => !known('classes', b));
  assert.equal(bad.length, 0, missing('classes missing from docs/themes.md section 11', bad.map((b) => '.' + b)));
});

test('every data-* attribute lull sets or styles is listed as stable or internal', () => {
  const attrs = [...attrsOf(source), ...designs.flatMap((d) => [...attrsOf(d.sel)])];
  const bad = attrs.filter((a) => !known('attrs', a));
  assert.equal(bad.length, 0, missing('data-* attributes missing from docs/themes.md section 11', bad));
});

test('every hook a built-in design styles exists in web/', () => {
  const bad = designs.flatMap((d) => [
    ...[...classesOf(d.sel)].filter((c) => !OWN.test(c) && !exists(c)).map((c) => `${d.name}: .${c}`),
    ...[...attrsOf(d.sel)].filter((a) => !exists(a)).map((a) => `${d.name}: ${a}`),
  ]);
  assert.equal(bad.length, 0, missing('rules for hooks web/ no longer has (they never match)', bad));
});

test('every hook the reference lists exists in web/', () => {
  const here = (n) => (n.endsWith('*') ? source.includes(n.slice(0, -1)) : exists(n));
  /* a block may appear only through its elements (.sys-pal__swatch) */
  const blockHere = (b) => here(b) || new RegExp(`(?<![\\w-])${b.replace(/-/g, '\\-')}(?:__|--)`).test(source);
  const bad = [
    ...[...stable.classes, ...internal.classes].filter((c) => !blockHere(c)).map((c) => '.' + c),
    ...[...stable.attrs, ...internal.attrs].filter((a) => !here(a)),
  ];
  assert.equal(bad.length, 0, missing('docs/themes.md section 11 lists hooks web/ no longer has', bad));
});

test('no hook is both stable and internal', () => {
  const both = [...[...stable.classes].filter((c) => internal.classes.has(c)).map((c) => '.' + c),
    ...[...stable.attrs].filter((a) => internal.attrs.has(a))];
  assert.equal(both.length, 0, missing('listed as both stable and internal', both));
});

/* the arguments of a selector's :has()es, parentheses matched */
function hasArgs(sel) {
  const out = [];
  for (let i = sel.indexOf(':has('); i >= 0; i = sel.indexOf(':has(', i + 1)) {
    let depth = 0, j = i + 4;
    for (; j < sel.length; j++) { if (sel[j] === '(') depth++; else if (sel[j] === ')' && --depth === 0) break; }
    out.push(sel.slice(i + 5, j));
  }
  return out;
}
// Chromium rechecks every :has() up the tree when an attribute named in one changes, so a :has() on the fitter's
// data-fit-out restyles the whole page at each step of a fit in a design with a :has() on the shell (docs/themes.md,
// Rows that fit themselves): a style that follows a part out reads its row's data-fit-gave
test('no style reads a part that gave way through :has()', () => {
  const bad = [shared, ...designs.map((d) => d.sel)].join('\n').split('\n')
    .filter((sel) => hasArgs(sel).some((a) => a.includes('data-fit-out')));
  assert.equal(bad.length, 0, missing('selectors with [data-fit-out] in a :has() (read the row\'s data-fit-gave)', bad));
});

/* the classes lull's markup sets: class="" (with the quoted names inside its ${}), className, classList */
function markupClasses() {
  const out = new Set(), word = /^[a-zA-Z_][\w-]*$/;
  const add = (text) => text.split(/\s+/).forEach((w) => { if (word.test(w)) out.add(w); });
  const quoted = (text) => { for (const s of text.matchAll(/'([^'\n]*)'/g)) add(s[1]); };
  for (const f of markup) {
    const src = read(f);
    for (const m of src.matchAll(/class(?:Name)?\s*=\s*"((?:[^"$]|\$\{(?:[^{}]|\{[^{}]*\})*\})*)"/g)) {
      add(m[1].replace(/\$\{(?:[^{}]|\{[^{}]*\})*\}/g, (e) => { quoted(e); return ' '; }));
    }
    for (const m of src.matchAll(/classList\.(?:add|remove|toggle|contains)\(([^)]*)\)/g)) quoted(m[1]);
    for (const m of src.matchAll(/className\s*\+?=\s*([^;\n]*)/g)) quoted(m[1]);
  }
  return out;
}
// Theme Studio and an import warn about a theme's rules for hooks lull moved, removed or keeps internal
// (web/designs/hooks.js): it carries this reference, and the names lull's markup sets that it doesn't list
test('designs/hooks.js carries the hook reference and the names lull sets', () => {
  const path = join(WEB, 'designs/hooks.js');
  const sorted = (s) => [...s].sort();
  const isListed = (kind, n) => known(kind, kind === 'classes' ? block(n) : n);
  const data = {
    stable: { classes: sorted(stable.classes), attrs: sorted(stable.attrs) },
    internal: { classes: sorted(internal.classes), attrs: sorted(internal.attrs) },
    classes: sorted([...markupClasses()].filter((c) => !isListed('classes', c))),
    attrs: sorted([...attrsOf(source)].filter((a) => !isListed('attrs', a))),
    prefixes: sorted(new Set(dynamic)),
  };
  const line = `  var HOOKS = ${JSON.stringify(data)};`;
  const have = readFileSync(path, 'utf8');
  const want = have.split('\n').map((l) => (l.startsWith('  var HOOKS = ') ? line : l)).join('\n');
  if (process.env.LULL_WRITE_PACKAGES) writeFileSync(path, want);
  assert.ok(readFileSync(path, 'utf8') === want && want.includes(`\n${line}\n`),
    'web/designs/hooks.js: HOOKS is stale: run LULL_WRITE_PACKAGES=1 node --test tests/web/theme-hooks.test.mjs');
});

test('every hook lull moved is in docs/themes.md, Hooks that changed', () => {
  const { MOVED } = createRequire(import.meta.url)('../../web/designs/hooks.js');
  const at = docs.indexOf('### Hooks that changed'), part = at < 0 ? '' : docs.slice(at, docs.indexOf('\n## ', at));
  const bad = MOVED.filter((m) => !part.includes(m.since) || !part.includes(m.name)).map((m) => `${m.since} ${m.name}`);
  assert.equal(bad.length, 0, missing('moves docs/themes.md section 9, Hooks that changed, doesn\'t list', bad));
});
