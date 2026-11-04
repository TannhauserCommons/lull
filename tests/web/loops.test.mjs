// A theme's loops that never end and name no rate (web/designs/loops.js; docs/themes.md, Drawing cheaply): Theme Studio
// and importing a theme find them, suggest the guide's rate, and add it where the loop is written. The built-in designs
// and the docs' examples name every rate, so they show what the check asks of a listener's theme.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const L = createRequire(import.meta.url)('../../web/designs/loops.js');
const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const read = (p) => readFileSync(join(ROOT, p), 'utf8');
const components = read('web/components.css');
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
const brief = (hints) => hints.map((h) => `${h.selector} ${h.animation} ${h.fps}${h.sure ? '' : '?'}`);

const PETALS = `/* falling petals */
body::before, body::after { content: ""; position: fixed; top: -5vh; width: 14px; height: 10px; border-radius: 70% 0;
  background: #f4a7c0; animation: petal-fall 9s linear infinite; }
body::after { left: 70%; animation-duration: 13s; }
@keyframes petal-fall { to { transform: translate(-6vw, 110vh) rotate(320deg); } }
`;

test('a loop that never ends and names no rate is found, with the rate the guide suggests and why', () => {
  const css = `${PETALS}
.glow::after { animation: glow 3s ease-in-out infinite alternate }
@media (min-width: 600px) { @supports (display: grid) { .caret { animation: blink 1s steps(1) infinite; } } }
.is-playing-now .bars i { animation-name: bounce; animation-duration: .8s; animation-iteration-count: infinite; }
.mystery { animation: from-elsewhere 4s infinite; }
.two { animation: glow 2s ease 1, petal-fall 9s linear infinite; }
@keyframes glow { 50% { opacity: .4; transform: scale(1.1); } }`;
  assert.deepEqual(brief(L.find(css)), [
    'body::before, body::after petal-fall 20',
    '.glow::after glow 10',
    '.caret blink 10',
    '.is-playing-now .bars i bounce 30',
    '.mystery from-elsewhere 20?',
    '.two petal-fall 20',
  ]);
  const [petals] = L.find(css);
  assert.equal(petals.line, 3);
  assert.equal(petals.why, 'it moves');
  assert.match(L.cost(1), /165 times a second/);
  assert.match(L.advice(petals), /^It moves, so 20 a second suits it\.$/);
  assert.match(L.advice(L.find(css)[4]), /can't tell what it shows, so it suggests 20/);
});

test('a loop that ends, or names its rate there or on the same selector elsewhere, is left alone', () => {
  const css = `.a { animation: spin 2s linear infinite; --loop-fps: 20; }
.b { animation: spin 2s linear infinite; }
.b { --loop-fps: 10; }
:root[data-design="skydeck"]  .c>.d::before { animation: spin 2s infinite; }
:root[data-design='skydeck'] .c > .d:before { --loop-fps: 30 }
.e { animation: pop .3s ease-out; }  .f { animation: none; }  .g { content: "infinite"; }
/* .h { animation: spin 1s infinite; } */
@keyframes spin { to { transform: rotate(1turn); } }`;
  assert.deepEqual(brief(L.find(css)), []);
  // --loop-fps is set per element: a rate on another element doesn't pace this one
  assert.deepEqual(brief(L.find('.box { --loop-fps: 20; } .box i { animation: spin 2s infinite; }')), ['.box i spin 20?']);
});

test('a rate the page sets already paces a theme\'s loop on the same selector: lull\'s, or the design it extends', () => {
  // components.css paces the playing bars; skydeck paces its sky, written :root[data-design="skydeck"] .sky-specks::before
  assert.deepEqual(brief(L.find('.is-playing-now .eq i { animation: my-eq .7s ease-in-out infinite alternate; }', { also: [components] })), []);
  const skydeck = read('web/designs/skydeck/theme.css');
  const sky = /:root\[data-design="skydeck"\] ([^{]*?)\s*\{[^}]*sd-twinkle[^}]*--loop-fps/.exec(skydeck);
  assert.ok(sky, 'skydeck paces its twinkle');
  assert.deepEqual(brief(L.find(`${sky[1]} { animation: my-twinkle 9s ease-in-out infinite alternate; }`, { also: [components, skydeck], base: 'skydeck' })), []);
  assert.equal(L.find(`${sky[1]} { animation: my-twinkle 9s infinite; }`, { also: [components] }).length, 1, 'without skydeck beside it');
});

test('adding the rates paces every loop and changes nothing else', () => {
  const css = `${PETALS}.é-dot::after { animation: glow 3s infinite alternate }\n.last { animation: blink 1s steps(1) infinite }\n@keyframes glow { to { opacity: 0 } }\n`;
  const hints = L.find(css);
  assert.equal(hints.length, 3);
  const fixed = L.apply(css, hints);
  assert.deepEqual(L.find(fixed), []);
  assert.equal(fixed, css
    .replace('petal-fall 9s linear infinite; }', 'petal-fall 9s linear infinite; --loop-fps: 20; }')
    .replace('glow 3s infinite alternate }', 'glow 3s infinite alternate; --loop-fps: 10; }')
    .replace('steps(1) infinite }', 'steps(1) infinite; --loop-fps: 10; }'));
  // one at a time, as Theme Studio adds them
  assert.deepEqual(L.find(L.apply(css, [hints[1]])).map((h) => h.animation), ['petal-fall', 'blink']);
});

test('the built-in designs and the docs\' examples name every loop\'s rate', () => {
  for (const d of DESIGNS) {
    assert.deepEqual(brief(L.find(read(`web/designs/${d}/theme.css`), { also: [components], base: d })), [], d);
  }
  const docs = read('docs/themes.md'), blocks = [...docs.matchAll(/```css\n([\s\S]*?)```/g)].map((m) => m[1]);
  assert.ok(blocks.some((b) => /infinite/.test(b)), 'the docs show a loop');
  blocks.forEach((b, i) => assert.deepEqual(brief(L.find(b, { also: [components] })), [], `docs/themes.md css block ${i + 1}`));
  for (const id of readdirSync(join(ROOT, 'docs/theme-examples'))) {
    assert.deepEqual(brief(L.find(read(`docs/theme-examples/${id}/theme.css`), { also: [components] })), [], id);
  }
});
