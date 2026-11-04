// Stepped easings for loops that never end (docs/themes.md, Drawing cheaply). A loop changes only as many times a second
// as its motion needs, and its easing is kept: the steps sample the curve. Each is a CSS `linear()` staircase, n steps
// over one iteration, the first step at the curve's start and the last at its end, written as a custom property:
//   --loop-<curve>-<n>: linear(...)
// so a loop uses `animation-timing-function: var(--loop-ease-in-out-55)` (5.5 s at 10 changes a second). The CSS keeps
// what this prints; `--check` reads the stylesheets and fails on any --loop-<curve>-<n> that differs from it.
//
//   node scripts/loop-easings.mjs ease-in-out 55 70     print those two
//   node scripts/loop-easings.mjs --check web            check every stylesheet under web/
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';

// the curves loops use: CSS's ease-in-out, and lull's --ease-in-out token (tokens.css)
const CURVES = { 'ease-in-out': [0.42, 0, 0.58, 1], 'in-out': [0.65, 0, 0.35, 1] };

function bezier([x1, y1, x2, y2]) {
  const at = (a, b, t) => 3 * a * t * (1 - t) ** 2 + 3 * b * t * t * (1 - t) + t ** 3;
  return (x) => {
    let lo = 0, hi = 1;
    for (let i = 0; i < 50; i++) { const m = (lo + hi) / 2; if (at(x1, x2, m) < x) lo = m; else hi = m; }
    return at(y1, y2, (lo + hi) / 2);
  };
}
const num = (v, d) => String(+v.toFixed(d));

export function staircase(curve, n) {
  const ease = bezier(CURVES[curve]), stops = [];
  for (let i = 0; i < n; i++) {
    const v = num(n === 1 ? 0 : ease(i / (n - 1)), 4);
    stops.push(`${v} ${num((i / n) * 100, 3)}%`, `${v} ${num(((i + 1) / n) * 100, 3)}%`);
  }
  return `linear(${stops.join(', ')})`;
}

const args = process.argv.slice(2);
if (args[0] === '--check') {
  const files = [];
  (function walk(d) { for (const f of readdirSync(d)) { const p = join(d, f); if (statSync(p).isDirectory()) { if (f !== 'vendor' && f !== 'node_modules') walk(p); } else if (p.endsWith('.css')) files.push(p); } })(args[1] || 'web');
  let bad = 0, seen = 0;
  for (const f of files) {
    for (const m of readFileSync(f, 'utf8').matchAll(/--loop-([a-z-]+?)-(\d+):\s*([^;]+);/g)) {
      const [, curve, n, value] = m;
      if (!CURVES[curve]) { console.error(`${f}: --loop-${curve}-${n}: no curve "${curve}" (scripts/loop-easings.mjs)`); bad++; continue; }
      seen++;
      if (value.trim() !== staircase(curve, +n)) { console.error(`${f}: --loop-${curve}-${n} differs from scripts/loop-easings.mjs; print it again`); bad++; }
    }
  }
  console.log(`${seen} stepped easings checked, ${bad} wrong`);
  process.exit(bad ? 1 : 0);
}
const [curve, ...ns] = args;
if (!CURVES[curve] || !ns.length) { console.error('usage: node scripts/loop-easings.mjs <' + Object.keys(CURVES).join('|') + '> <steps>... | --check [dir]'); process.exit(2); }
for (const n of ns) console.log(`  --loop-${curve}-${n}: ${staircase(curve, +n)};`);
