// A stable hook is a promise to theme authors (docs/themes.md, ADR-0004): a theme that styles it keeps working from one
// release to the next. stable-hooks.json beside this file holds every name docs/themes.md section 11 has ever listed as
// stable; each stays stable, in the reference and in web/designs/hooks.js (written from it), unless lull says where it
// went: a rule naming it gets a `moved` hint from LullHooks.find (its MOVED list, docs/themes.md section 9), so Theme
// Studio and an import tell the theme's author. A name newly listed stable joins the list:
// LULL_WRITE_PACKAGES=1 node --test tests/web/stable-hooks.test.mjs. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const ROOT = join(here, '..', '..');
const LEDGER = join(here, 'stable-hooks.json');
const KINDS = ['classes', 'attrs'];

const ledger = JSON.parse(readFileSync(LEDGER, 'utf8'));
const hooksJs = join(ROOT, 'web/designs/hooks.js');
const Hooks = createRequire(import.meta.url)(hooksJs);
/* web/designs/hooks.js's own copy of the reference: its HOOKS line */
const inHooksJs = JSON.parse(/var HOOKS = (\{.*\});/.exec(readFileSync(hooksJs, 'utf8'))[1]).stable;

/* docs/themes.md section 11 above "### Internal": the stable part of the reference (as tests/web/theme-hooks.test.mjs reads it) */
const docs = readFileSync(join(ROOT, 'docs/themes.md'), 'utf8');
const from = docs.indexOf('\n## 11. ');
const next = docs.indexOf('\n## ', from + 1);
const ref = from < 0 ? '' : docs.slice(from, next < 0 ? undefined : next);
const spans = (ref.slice(0, ref.indexOf('\n### Internal')).match(/`[^`\n]+`/g) || []).join(' ');
const inDocs = {
  classes: [...new Set((spans.match(/(?<![\w-])\.[a-zA-Z_][\w-]*\*?/g) || []).map((c) => c.slice(1)))],
  attrs: [...new Set(spans.match(/data-[a-z][a-z0-9-]*\*?/g) || [])],
};

/* a name as a theme writes it: one a wildcard (is-*) stands for, and the rule a theme styles it with */
const sample = (name) => (name.endsWith('*') ? name.slice(0, -1) + 'x' : name);
const listed = (list, name) => list.some((n) => n === name || (n.endsWith('*') && sample(name).startsWith(n.slice(0, -1))));
const rule = (kind, name) => `${kind === 'classes' ? '.' + sample(name) : '[' + sample(name) + ']'} { color: red; grid-area: a; order: 1; }`;
const moved = (kind, name) => Hooks.find(rule(kind, name)).some((h) => h.kind === 'moved');
const say = (kind, name) => (kind === 'classes' ? '.' + name : name);

test('the reference is where the stable hooks are read from', () => {
  assert.ok(spans.length > 1000, 'docs/themes.md needs "## 11. Hook reference" with an "### Internal" part');
});

test('every hook ever listed stable is still stable, or lull says where it moved', () => {
  const gone = [];
  for (const kind of KINDS) {
    for (const name of ledger[kind]) {
      if (moved(kind, name)) continue;
      const where = [!listed(inDocs[kind], name) && 'docs/themes.md section 11', !listed(inHooksJs[kind], name) && 'web/designs/hooks.js'].filter(Boolean);
      if (where.length) gone.push(`${say(kind, name)} (not stable in ${where.join(', ')})`);
    }
  }
  assert.deepEqual(gone, [], 'a stable hook left without a MOVED entry (web/designs/hooks.js, docs/themes.md section 9) saying what to use instead');
});

test('every hook listed stable now is in the list of promises', () => {
  const missing = KINDS.flatMap((kind) => inDocs[kind].filter((name) => !ledger[kind].includes(name)).map((name) => [kind, name]));
  if (missing.length && process.env.LULL_WRITE_PACKAGES) {
    const out = Object.fromEntries(KINDS.map((kind) => [kind, [...new Set([...ledger[kind], ...missing.filter(([k]) => k === kind).map(([, n]) => n)])].sort()]));
    writeFileSync(LEDGER, JSON.stringify(out, null, 2) + '\n');
    return;
  }
  assert.deepEqual(missing.map(([kind, name]) => say(kind, name)), [],
    'tests/web/stable-hooks.json: add them with LULL_WRITE_PACKAGES=1 node --test tests/web/stable-hooks.test.mjs');
});
