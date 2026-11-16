// A key's accessible name is what it does ("Shuffle"); its shortcut goes in aria-keyshortcuts ("S"), so a screen reader
// says the name and then the shortcut, instead of "Shuffle, left paren, S". This reads the shipped markup itself (lull's
// pages and screens, every built-in design's slots), so a key that only shows in one layout (Now Playing's compact
// hero, a phone) is held to it too; the painter's names are the gates' (tests/ui/gates/player-paint.spec.mjs).
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const WEB = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'web');
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
function walk(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules') return [];
    return statSync(path).isDirectory() ? walk(path) : [relative(WEB, path).replace(/\\/g, '/')];
  });
}
// written by lull-themes from each design's theme.json and slots, which are read here as they are written
const generated = (f) => DESIGNS.some((d) => f === `designs/${d}.js`);
const markup = walk(WEB).filter((f) => /\.(html|js)$/.test(f) && !generated(f) && !f.endsWith('.min.js'));

// a name ending in a key in brackets: "(S)", "(Space)", "(Down arrow)", "(Shift+N)"
const SHORTCUT = /\s\((?:[A-Za-z]|Space|Esc|Escape|(?:Up|Down|Left|Right) arrow|Shift\+\S+|Ctrl\+\S+)\)$/;
// aria-label="..." (markup and templates) and setAttribute / setAttr(el, 'aria-label', '...' + ' (R)') (scripts)
const LABELS = [/aria-label="([^"\n]*)"/g, /aria-label='([^'\n]*)'/g, /['"]aria-label['"]\s*,\s*([^)\n]*)\)/g];

test('no key in lull\'s markup or a design\'s slots names its shortcut in its accessible name', () => {
  const bad = [];
  for (const f of markup) {
    const text = readFileSync(join(WEB, f), 'utf8');
    for (const re of LABELS) {
      for (const m of text.matchAll(re)) {
        const label = m[1].replace(/['"`]\s*$/, '').trim();
        if (SHORTCUT.test(label)) bad.push(`${f}:${text.slice(0, m.index).split('\n').length}: ${label}`);
      }
    }
  }
  assert.deepEqual(bad, [], 'put the shortcut in aria-keyshortcuts, and keep the name the key\'s words');
});
