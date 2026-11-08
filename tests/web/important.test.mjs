// lull's own `!important` rules hold only what must hold whatever a design draws, each with its reason, listed in
// docs/themes.md ("What lull holds fixed, and why", section 3): ADR-0101, section 4. A theme can't beat one (an important
// declaration in an earlier layer wins over one in a later layer), so each is a place no theme can change. This fails a
// new `!important` in lull's CSS that the list doesn't name, and a listed one that is gone. lull's CSS is web/*.css,
// web/screens/*.css, web/desktop/*.css and the <style> blocks in their pages; never web/designs/ (designs are themes)
// or web/packs/.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const WEB = join(ROOT, 'web');
const DIRS = ['', 'screens', 'desktop'];

// each file of lull's own CSS, by its path under web/, with its CSS (a page's <style> blocks, one after another)
const sheets = DIRS.flatMap((dir) => readdirSync(join(WEB, dir)).flatMap((name) => {
  const at = dir ? `${dir}/${name}` : name, text = () => readFileSync(join(WEB, dir, name), 'utf8');
  if (name.endsWith('.css')) return [{ at, css: text() }];
  if (name.endsWith('.html')) return [{ at, css: [...text().matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join('\n') }];
  return [];
}));

// every important declaration in a sheet: "<file> | <selector> | <property>", the selector with its whitespace folded.
// Comments and strings are read past, so a brace or a semicolon in them doesn't count; at-rules (@media, @layer) only
// nest the rules inside them
function importants(at, css) {
  const out = [], stack = [];
  let text = '';
  const declarations = (block, selector) => {
    for (const d of block.split(/;(?=(?:[^"']*["'][^"']*["'])*[^"']*$)/)) {   // a semicolon in a string ends nothing
      const m = /^\s*([-\w]+)\s*:([\s\S]*)!\s*important\s*$/i.exec(d);
      if (m) out.push(`${at} | ${selector} | ${m[1].toLowerCase()}`);
    }
  };
  for (let i = 0; i < css.length; i++) {
    const c = css[i];
    if (c === '/' && css[i + 1] === '*') { i = css.indexOf('*/', i + 2) + 1; if (i === 0) break; continue; }
    if (c === '"' || c === "'") { const end = css.indexOf(c, i + 1); text += css.slice(i, end + 1); i = end; continue; }
    if (c === '{') {
      // what came before the brace since the last declaration: the rule's selector or the at-rule's prelude
      const cut = text.lastIndexOf(';'), prelude = text.slice(cut + 1).replace(/\s+/g, ' ').trim();
      if (stack.length && !stack[stack.length - 1].at) declarations(text.slice(0, cut + 1), stack[stack.length - 1].prelude);
      stack.push({ prelude, at: prelude.startsWith('@') });
      text = '';
    } else if (c === '}') {
      const top = stack.pop();
      if (top && !top.at) declarations(text, top.prelude);
      text = '';
    } else text += c;
  }
  return out;
}

// the list in docs/themes.md: a table whose rows say the file, the rule (its selector in backticks) and what it holds
// (its properties in backticks)
function listed() {
  const doc = readFileSync(join(ROOT, 'docs', 'themes.md'), 'utf8');
  const start = doc.indexOf('**What lull holds fixed, and why.**');
  assert.ok(start >= 0, 'docs/themes.md has no "What lull holds fixed, and why" list');
  const lines = doc.slice(start).split('\n'), first = lines.findIndex((l) => l.startsWith('|'));
  const rows = [];
  for (let i = first + 2; first >= 0 && i < lines.length && lines[i].startsWith('|'); i++) rows.push(lines[i]);   // past the head and its rule
  return rows.flatMap((row) => {
    const [file, rule, holds, why] = row.slice(1, -1).split(' | ').map((s) => s.trim());
    assert.ok(why && why.length > 20, `docs/themes.md: ${rule} says why it holds`);
    const selector = /^`(.*)`$/.exec(rule)?.[1];
    assert.ok(selector, `docs/themes.md: the rule ${rule} in backticks`);
    return [...holds.matchAll(/`([-\w]+)`/g)].map((m) => `${file.replace(/`/g, '')} | ${selector.replace(/\s+/g, ' ')} | ${m[1]}`);
  });
}

test('lull\'s own !important rules are the ones docs/themes.md lists, each with its reason', () => {
  const found = new Set(sheets.flatMap((s) => importants(s.at, s.css)));
  const list = new Set(listed());
  assert.ok(found.size > 0, 'no !important found: the reader is broken');
  const unlisted = [...found].filter((k) => !list.has(k));
  const gone = [...list].filter((k) => !found.has(k));
  assert.deepEqual(unlisted, [], 'a new !important in lull\'s CSS: hold only what must hold whatever a design draws, and list it with its reason in docs/themes.md, section 3');
  assert.deepEqual(gone, [], 'docs/themes.md lists an !important lull\'s CSS no longer has: take it off the list');
});

test('the reader finds an important declaration, and none in a comment or a string', () => {
  const css = '/* a { color: red !important; } */ @media (x) { .a, .b { color: red; margin: 0 !important } } .c { content: "{ x; y !important }"; display: none ! important; }';
  assert.deepEqual(importants('t.css', css), ['t.css | .a, .b | margin', 't.css | .c | display']);
});
