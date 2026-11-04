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

test('a rate or a hold the page sets on the same selector counts for a theme\'s loop: lull\'s, or the design it extends', () => {
  // a rate named on the selector elsewhere, written :root[data-design="skydeck"] in the design the theme extends
  const base = ':root[data-design="skydeck"] .sky-specks::after { --loop-fps: 10; }';
  assert.deepEqual(brief(L.find('.sky-specks::after { animation: my-twinkle 9s ease-in-out infinite alternate; }', { also: [components, base], base: 'skydeck' })), []);
  assert.equal(L.find('.sky-specks::after { animation: my-twinkle 9s infinite; }', { also: [components] }).length, 1, 'without it beside it');
  // lull's own loops step in their animation, so a theme that replaces the animation replaces its steps: unpaced again
  assert.ok(/\.eq i[^{]*\{[^}]*--loop-ease-in-out-\d+[^}]*--loop-play/.test(components), 'components.css steps and holds the playing bars');
  assert.deepEqual(brief(L.find('.is-playing-now .eq i { animation: my-eq .7s ease-in-out infinite alternate; }', { also: [components] })), ['.is-playing-now .eq i my-eq 30']);
  // stepped where it is declared and held there, or held on the same selector elsewhere
  assert.deepEqual(brief(L.find(`.a { animation: spin 2s steps(40) infinite; animation-play-state: var(--loop-play, running); }
.b { animation: glow 5.5s var(--loop-ease-in-out-55) infinite alternate; }
.b { animation-play-state: var(--loop-play, running); }
.c { animation: spin 2s steps(40) infinite; }
.d { animation: glow 3s ease-in-out infinite; animation-play-state: var(--loop-play, running); }
@keyframes spin { to { transform: rotate(1turn); } } @keyframes glow { to { opacity: 0; } }`)), ['.c spin 10', '.d glow 10']);
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

// what a loop animates besides transform and opacity, each with the cheaper way; and glass it sits in
const kinds = (hints, kind) => hints.filter((h) => h.kind === kind);
const loop = (sel, kf, body) => `${sel} { animation: ${kf} 2s ease-in-out infinite alternate; --loop-fps: 10; }\n@keyframes ${kf} { to { ${body} } }\n`;

test('a loop that animates more than transform and opacity is named, with the cheaper way for what it changes', () => {
  const css = [
    loop('.glow', 'glow', 'box-shadow: 0 0 12px pink; filter: brightness(1.2);'),
    loop('.stripes', 'slide', 'background-position: 40px 0;'),
    loop('.walk', 'walk', 'left: 100px;'),
    loop('.grow', 'grow', 'width: 120px;'),
    loop('.blush', 'blush', 'background-color: pink;'),
    loop('.wipe', 'wipe', 'clip-path: inset(0 50% 0 0);'),
    loop('.text', 'letters', 'letter-spacing: 2px;'),
    loop('.fine', 'fine', 'transform: translateY(4px) scale(1.1); opacity: .5; translate: 2px; rotate: 4deg; scale: 1.1;'),
    '.borrowed { animation: from-elsewhere 3s infinite; --loop-fps: 10; }\n',
  ].join('');
  const found = kinds(L.find(css), 'repaint');
  assert.deepEqual(found.map((h) => `${h.selector}: ${h.props.join(' ')}`), [
    '.glow: box-shadow filter', '.stripes: background-position', '.walk: left', '.grow: width', '.blush: background-color', '.wipe: clip-path', '.text: letter-spacing',
  ]);
  assert.deepEqual(found.map((h) => h.fix.split(' ').slice(0, 4).join(' ')), [
    'For a glow, draw', 'To slide a pattern,', 'To move it, use', 'To grow it, use', 'To change its colour,', 'To reveal it, move', 'Rebuild it with transform',
  ]);
  assert.ok(found.every((h) => !h.insert), 'nothing it rewrites by itself');
  assert.equal(L.apply(css, found), css);
  // lull's own keyframes, when a theme runs them
  assert.deepEqual(kinds(L.find('.x { animation: breathe 2s infinite; --loop-fps: 10; }', { also: [components] }), 'repaint'), []);
});

test('a loop inside glass is named from the CSS; one that may pass under it is said to be unknown, never guessed', () => {
  const css = `.panel { backdrop-filter: blur(12px); }
.panel .twinkle::after { animation: tw 3s infinite alternate; --loop-fps: 10; }
.panel > .dot { animation: tw 3s infinite alternate; --loop-fps: 10; }
.pane { -webkit-backdrop-filter: blur(4px); animation: tw 3s infinite alternate; --loop-fps: 10; }
.panel + .beside { animation: tw 3s infinite; --loop-fps: 10; }
.flat { backdrop-filter: none; } .flat .x { animation: tw 3s infinite; --loop-fps: 10; }
@keyframes tw { to { opacity: .4 } }`;
  assert.deepEqual(kinds(L.find(css), 'glass').map((h) => `${h.selector} in ${h.glass}${h.itself ? ' (itself)' : ''}`),
    ['.panel .twinkle::after in .panel', '.panel > .dot in .panel', '.pane in .pane (itself)']);
  assert.deepEqual(kinds(L.find(css), 'glass-maybe'), [], 'Theme Studio looks in its previews instead');
  const [maybe] = kinds(L.find(css, { unsure: true }), 'glass-maybe');
  assert.deepEqual(maybe.loops.map((l) => l.selector), ['.panel + .beside', '.flat .x']);
  assert.deepEqual(maybe.glass, ['.panel', '.pane']);
  assert.deepEqual(kinds(L.find('.a { animation: tw 3s infinite; --loop-fps: 10; }', { unsure: true }), 'glass-maybe'), [], 'no glass, nothing to wonder');
  // the glass of the design a theme extends, as its cascade leaves it: Glasshouse blurs .ctx and the cards live, and bakes
  // the player bar's (backdrop-filter: none after)
  const glasshouse = read('web/designs/glasshouse/theme.css');
  const inGlasshouse = (sel) => kinds(L.find(`${sel} { animation: tw 3s infinite; --loop-fps: 10; } @keyframes tw { to { opacity: 0 } }`,
    { also: [components, glasshouse], base: 'glasshouse' }), 'glass').map((h) => h.glass);
  assert.deepEqual(inGlasshouse('.ctx i::before'), ['.ctx']);
  assert.deepEqual(inGlasshouse('.card > .x'), ['.card']);
  assert.deepEqual(inGlasshouse('.pbar i::before'), [], 'baked');
});

test('a loop kept as the fallback that @supports turns off is left alone', () => {
  const pulse = '.tray .led { animation: pulse 2s infinite alternate; --loop-fps: 20; }\n@keyframes pulse { to { box-shadow: 0 0 6px red; } }\n';
  assert.deepEqual(L.find(pulse + '@supports (anchor-name: --a) { .tray .led.on { animation: none; } }'), []);
  assert.deepEqual(L.find(pulse + '@supports (anchor-name: --a) { .tray .led { animation-name: none; } }'), []);
  assert.equal(L.find(pulse + '@supports (anchor-name: --a) { .tray .other { animation: none; } }').length, 1, 'another element');
  assert.equal(L.find(pulse + '.tray .led.on { animation: none; }').length, 1, 'not a fallback: switched off everywhere it matches');
});

// A reason a theme words for itself on a state whose reason changes from song to song (the failed song's "no peer
// answered", wrong whenever YouTube or the library was the trouble): named, and replaced by lull's own words
// (--status-text) where the rule draws inside the player.
const CANNED = '.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  no peer answered"; color: var(--heart); }\n';

test('a reason a theme words for itself on a state whose reason varies is named, and lull\'s own words offered', () => {
  const [h] = L.find(CANNED);
  assert.equal(h.kind, 'status-words');
  assert.equal(h.words, '⚠ no peer answered');
  assert.equal(h.state, 'failed');
  assert.equal(h.line, 1);
  const fixed = L.apply(CANNED, [h]);
  assert.equal(fixed, '.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  " var(--status-text); color: var(--heart); }\n', 'its own sign kept');
  assert.deepEqual(L.find(fixed), []);
  assert.match(L.say('status-words', 1).lead, /var\(--status-text\)/);

  const words = (css) => L.find(css).filter((x) => x.kind === 'status-words').map((x) => `${x.state}: ${x.words}${x.insert ? ' -> ' + x.insert : ' (no fix)'}`);
  assert.deepEqual(words([
    "#mini[data-status='jellyfin-error'] .mini__chip::before { content: 'library offline' !important }",
    '.pbar:is([data-status="stalled"], [data-status="finding"]) .pbar__seek::before { content: "waiting on a peer"; }',
    '[data-dk-player][data-status="buffering"]::after { content: "buffering from Soulseek" }',
    '.app:has(> .pbar[data-status="failed"]) .now-dev__page::after { content: "YouTube said no"; }',
    '.pbar[data-status="queued"] + .x::before { content: "queued at a peer"; }',
    '.pbar[data-status="failed"] .pbar__seek::before { content: "no copy " attr(data-x); }',
    '.pbar[data-status="streaming"] .pbar__seek::before { content: "streaming from a peer"; }',
  ].join('\n')), [
    "jellyfin-error: library offline -> var(--status-text)",
    'stalled: waiting on a peer (no fix)',                          // in an :is(): lull can't tell where it draws
    'buffering: buffering from Soulseek -> var(--status-text)',
    'failed: YouTube said no (no fix)',                              // drawn on Now Playing, outside the player
    'queued: queued at a peer (no fix)',                             // on a sibling of the bar
    'failed: no copy (no fix)',                                      // more than one string to replace
    'streaming: streaming from a peer -> var(--status-text)',       // YouTube streams too
  ]);
});

test('words that name no cause, and states whose words don\'t vary, are the theme\'s', () => {
  assert.deepEqual(L.find([
    '.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  couldn\'t play this song"; }',
    '.pbar:is([data-status="failed"], [data-status="choose"]) .pbar__seek::before { content: "\\25cb  stopped"; }',
    '.app:has(> .pbar[data-status="failed"]) .now-dev__page::after { content: "all sources failed"; }',
    '.pbar[data-status="library"] .pbar__seek::before { content: "\\25cf  library"; }',
    '.pbar[data-status="failed"] .pbar__seek::before { content: "\\26A0  " var(--status-text); }',
  ].join('\n')), []);
});
