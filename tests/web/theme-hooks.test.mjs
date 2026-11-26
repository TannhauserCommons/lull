// docs/themes.md is the theme API (ADR-0004). Its hook reference (section 11) names every class and data-* attribute
// lull's CSS styles or its markup sets, each either stable or internal, so the docs can't fall behind web/:
// - every class block in web/'s own CSS, and every hook a built-in design styles, is listed in one of the two;
// - every data-* attribute web/ sets or styles is listed in one of the two;
// - every hook a built-in design names exists in web/ (a rule for a hook that is gone never matches);
// - every name the reference lists still exists in web/.
// A stable hook is a promise to theme authors: a theme that styles it keeps working from one release to the next. So
// every name the reference has ever listed stable (tests/web/stable-hooks.json) stays stable unless lull says where it
// went (a MOVED entry, which Theme Studio and an import tell the theme's author of). scripts/hooks.mjs reads web/ and
// the reference, and writes from them web/designs/hooks.js and that list (`mise run hooks`); this only checks.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { join } from 'node:path';
import {
  HOOKS_JS, LEDGER, OWN, ROOT, attrsOf, block, classesOf, cut, designs, docs, exists, hooksFile, internal, known, ledgerFile,
  listed, ref, shared, source, stable,
} from '../../scripts/hooks.mjs';

const Hooks = createRequire(import.meta.url)(HOOKS_JS);
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

// Theme Studio and an import warn about a theme's rules for hooks lull moved, removed or keeps internal from this copy
test('web/designs/hooks.js carries the hook reference and the names lull sets', () => {
  assert.ok(readFileSync(HOOKS_JS, 'utf8') === hooksFile(), 'web/designs/hooks.js: HOOKS is stale: run mise run hooks');
});

test('every hook listed stable now is in the list of promises', () => {
  assert.ok(readFileSync(LEDGER, 'utf8') === ledgerFile(),
    'tests/web/stable-hooks.json lacks hooks docs/themes.md section 11 now lists stable: run mise run hooks');
});

/* a name as a theme writes it (one a wildcard, is-*, stands for), and whether a rule styling it gets a `moved` hint */
const sample = (name) => (name.endsWith('*') ? name.slice(0, -1) + 'x' : name);
const moved = (kind, name) => Hooks.find(`${kind === 'classes' ? '.' + sample(name) : '[' + sample(name) + ']'} { color: red; grid-area: a; order: 1; }`)
  .some((h) => h.kind === 'moved');
test('every hook ever listed stable is still stable, or lull says where it moved', () => {
  const ledger = JSON.parse(readFileSync(LEDGER, 'utf8'));
  const gone = ['classes', 'attrs'].flatMap((kind) => ledger[kind]
    .filter((name) => !moved(kind, name) && !listed(stable[kind], sample(name)))
    .map((name) => (kind === 'classes' ? '.' + name : name)));
  assert.deepEqual(gone, [], 'stable hooks docs/themes.md section 11 no longer lists stable, with no MOVED entry (web/designs/hooks.js, docs/themes.md section 9) saying what to use instead');
});

// The theme the theme-API gate keeps working (tests/ui/fixtures/themes/theme-api-v1, gates/theme-api.spec.mjs) is the
// promise to theme authors, so it names stable hooks only: none lull moved, has no more, or keeps internal
test('the theme-API gate\'s theme styles stable hooks only', () => {
  const css = readFileSync(join(ROOT, 'tests/ui/fixtures/themes/theme-api-v1/theme.css'), 'utf8');
  assert.deepEqual(Hooks.find(css).map((h) => `${h.kind} ${h.name}`), []);
});

test('every hook lull moved is in docs/themes.md, Hooks that changed', () => {
  const at = docs.indexOf('### Hooks that changed'), part = at < 0 ? '' : docs.slice(at, docs.indexOf('\n## ', at));
  const bad = Hooks.MOVED.filter((m) => !part.includes(m.since) || !part.includes(m.name)).map((m) => `${m.since} ${m.name}`);
  assert.equal(bad.length, 0, missing('moves docs/themes.md section 9, Hooks that changed, doesn\'t list', bad));
});
