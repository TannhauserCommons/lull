// The theme API's hooks (ADR-0004, docs/themes.md): docs/themes.md section 11, the hook reference, is the one place a
// hook is classified, stable or internal, and everything else is written from it here:
// - web/designs/hooks.js's HOOKS line: the reference, with the classes and data-* attributes lull's markup sets that it
//   doesn't list and the prefixes lull builds names from, which Theme Studio and an import read to warn a theme that
//   styles a hook lull moved, has no more or keeps internal;
// - tests/web/stable-hooks.json: every name the reference has ever listed stable, the promises made to theme authors.
//   It only grows: a stable hook leaves it by hand, and only with a MOVED entry saying where it went (hooks.js, docs/themes.md
//   section 9).
// tests/web/theme-hooks.test.mjs (in `mise run check`) reads web/ and the reference through this module, fails while
// either file is stale, and checks the reference is complete and every promise is kept.
//
//   node scripts/hooks.mjs      (mise run hooks) write both from the reference and web/
import { readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const WEB = join(ROOT, 'web');
export const HOOKS_JS = join(WEB, 'designs/hooks.js');
export const LEDGER = join(ROOT, 'tests/web/stable-hooks.json');
export const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
export const OWN = /^(lull|sd|sl|pt|gh)-/;              // a built-in design's own classes (docs/themes.md, versioning)

function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') return [];
    return statSync(path).isDirectory() ? walk(path) : [relative(WEB, path).replace(/\\/g, '/')];
  });
}
const files = walk(WEB);
const read = (f) => readFileSync(join(WEB, f), 'utf8');
// a built-in design's pack.css (packs/<id>/pack.css, the looks of its easter eggs) is that design's, with its own classes
const designCss = (f) => DESIGNS.some((d) => f === `designs/${d}/theme.css` || f === `packs/${d}/pack.css`);
const generated = (f) => DESIGNS.some((d) => f === `designs/${d}.js`);    // theme.json + slots, written by lull-themes
const hooksJs = (f) => f === 'designs/hooks.js';                            // written here

const sharedCss = files.filter((f) => f.endsWith('.css') && !designCss(f));
const markup = files.filter((f) => /\.(js|html)$/.test(f) && !generated(f) && !hooksJs(f) && !f.endsWith('.min.js'));

/* the selectors of a stylesheet (comments and strings dropped, declarations skipped) */
export function selectors(css) {
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
export const classesOf = (sel) => new Set((sel.match(/\.[a-zA-Z_][\w-]*/g) || []).map((c) => c.slice(1)));
export const attrsOf = (text) => new Set(text.match(/data-[a-z][a-z0-9]*(?:-[a-z0-9]+)*/g) || []);
/* a BEM block: .trow__num and .trow--dense are .trow's; state classes (.is-*, .has-*) stand alone */
export const block = (c) => (/^(is|has)-/.test(c) ? c : c.replace(/(__|--).*$/, ''));

/* web/'s own selectors, each built-in design's, and everything lull's markup and CSS say */
export const shared = selectors(sharedCss.map(read).join('\n'));
export const designs = DESIGNS.map((d) => ({
  name: d, sel: selectors(read(`designs/${d}/theme.css`) + (files.includes(`packs/${d}/pack.css`) ? '\n' + read(`packs/${d}/pack.css`) : '')),
}));
export const source = markup.map(read).join('\n') + '\n' + shared;

/* names markup builds from a prefix (`now-ly--${key}`, 'c' + n): the prefix stands for all of them */
export const dynamic = [...source.matchAll(/([a-zA-Z][\w-]*?(?:-|__))(?:\$\{|['"]\s*\+)/g)].map((m) => m[1]);
const literal = (name) => new RegExp(`(?<![\\w-])${name.replace(/-/g, '\\-')}(?![\\w-])`).test(source);
export const exists = (name) => literal(name) || dynamic.some((p) => name.startsWith(p) && name.length > p.length);

/* the reference: section 11 of docs/themes.md; "### Internal" and below is internal, the rest stable */
export const docs = readFileSync(join(ROOT, 'docs/themes.md'), 'utf8');
const from = docs.indexOf('\n## 11. ');
const next = docs.indexOf('\n## ', from + 1);
export const ref = from < 0 ? '' : docs.slice(from, next < 0 ? undefined : next);
export const cut = ref.indexOf('\n### Internal');
function names(md) {
  const spans = (md.match(/`[^`\n]+`/g) || []).join(' ');
  return {
    classes: new Set((spans.match(/(?<![\w-])\.[a-zA-Z_][\w-]*\*?/g) || []).map((c) => c.slice(1))),
    attrs: new Set(spans.match(/data-[a-z][a-z0-9-]*\*?/g) || []),
  };
}
export const stable = names(ref.slice(0, cut));
export const internal = names(ref.slice(cut));
/* a name in a list of them, where a name ending * stands for every name it starts */
export const listed = (set, name) => set.has(name) || [...set].some((n) => n.endsWith('*') && name.startsWith(n.slice(0, -1)));
export const known = (kind, name) => listed(stable[kind], name) || listed(internal[kind], name);

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

/* web/designs/hooks.js as it should be: its HOOKS line written from the reference and the names lull's markup sets */
export function hooksFile() {
  const sorted = (s) => [...s].sort();
  const isListed = (kind, n) => known(kind, kind === 'classes' ? block(n) : n);
  const data = {
    stable: { classes: sorted(stable.classes), attrs: sorted(stable.attrs) },
    internal: { classes: sorted(internal.classes), attrs: sorted(internal.attrs) },
    classes: sorted([...markupClasses()].filter((c) => !isListed('classes', c))),
    attrs: sorted([...attrsOf(source)].filter((a) => !isListed('attrs', a))),
    prefixes: sorted(new Set(dynamic)),
  };
  const have = readFileSync(HOOKS_JS, 'utf8');
  if (!have.includes('\n  var HOOKS = ')) throw new Error('web/designs/hooks.js has no "  var HOOKS = " line to write');
  return have.split('\n').map((l) => (l.startsWith('  var HOOKS = ') ? `  var HOOKS = ${JSON.stringify(data)};` : l)).join('\n');
}

/* tests/web/stable-hooks.json as it should be: every name it holds, and each the reference lists stable now */
export function ledgerFile() {
  const have = JSON.parse(readFileSync(LEDGER, 'utf8'));
  const out = Object.fromEntries(['classes', 'attrs'].map((kind) => [kind, [...new Set([...have[kind], ...stable[kind]])].sort()]));
  return JSON.stringify(out, null, 2) + '\n';
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  for (const [path, want] of [[HOOKS_JS, hooksFile()], [LEDGER, ledgerFile()]]) {
    const changed = readFileSync(path, 'utf8') !== want;
    if (changed) writeFileSync(path, want);
    console.log(`${relative(ROOT, path).replace(/\\/g, '/')}: ${changed ? 'written' : 'up to date'}`);
  }
}
