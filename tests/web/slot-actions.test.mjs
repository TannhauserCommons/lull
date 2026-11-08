// A built-in design's slots name actions two ways, and a name nothing runs draws a button that does nothing:
// - data-act="..." (the Now Playing hero, Home's lead, the player): an action web/ registers, in app.js's table
//   (Object.assign(actions, { ... })) or by App.action('name', fn);
// - data-dk="..." (the mini player and the strip): a key in web/desktop/mini-keys.json, the list desktop.js runs and
//   lull-themes checks every slot against, a listener's too. A design's CSS that styles a key by another name never
//   matches.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..', '..');
const WEB = join(ROOT, 'web');
const DESIGNS = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];

function scripts(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (name === 'vendor' || name === 'node_modules' || name === 'designs') return [];
    return statSync(path).isDirectory() ? scripts(path) : name.endsWith('.js') ? [path] : [];
  });
}
const source = scripts(WEB).map((f) => readFileSync(f, 'utf8')).join('\n');
const app = readFileSync(join(WEB, 'app.js'), 'utf8');
const table = app.slice(app.indexOf('Object.assign(actions, {'), app.indexOf('\n  });', app.indexOf('Object.assign(actions, {')));
const registered = new Set([
  ...[...table.matchAll(/'([a-z][a-z0-9-]*)': function/g)].map((m) => m[1]),
  ...[...source.matchAll(/App\.action\('([a-z][a-z0-9-]*)'/g)].map((m) => m[1]),
]);
const keys = JSON.parse(readFileSync(join(WEB, 'desktop', 'mini-keys.json'), 'utf8'));

const slots = DESIGNS.flatMap((d) => {
  const dir = join(WEB, 'designs', d, 'slots');
  return readdirSync(dir).map((f) => ({ at: `${d}/slots/${f}`, html: readFileSync(join(dir, f), 'utf8') }));
});
const named = (attr) => slots.flatMap(({ at, html }) =>
  [...html.matchAll(new RegExp(`\\s${attr}="([^"]*)"`, 'g'))].map((m) => ({ at, name: m[1] })));

test('the action table is where this test looks for it', () => {
  for (const name of ['play', 'prev', 'next', 'shuffle', 'repeat', 'like', 'now-vol']) assert.ok(registered.has(name), name);
});

test('every data-act a built-in slot names is an action web/ runs', () => {
  const acts = named('data-act');
  assert.ok(acts.length > 10, 'the slots name their actions');
  assert.deepEqual(acts.filter((a) => !registered.has(a.name)).map((a) => `${a.at}: ${a.name}`), []);
});

test('every data-dk a built-in slot names, or its CSS styles, is a mini player key', () => {
  const dk = named('data-dk');
  assert.ok(dk.length > 10, 'the mini player slots name their keys');
  assert.deepEqual(dk.filter((k) => !(k.name in keys)).map((k) => `${k.at}: ${k.name}`), []);
  const styled = DESIGNS.flatMap((d) => [...readFileSync(join(WEB, 'designs', d, 'theme.css'), 'utf8').matchAll(/data-dk="([^"]*)"/g)]
    .map((m) => ({ at: `${d}/theme.css`, name: m[1] })));
  assert.deepEqual(styled.filter((k) => !(k.name in keys)).map((k) => `${k.at}: ${k.name}`), []);
});

// docs/themes.md section 15 lists the actions a theme's own button can name (data-act): each is one web/ runs, so a
// theme built from the list never draws a button that does nothing
test('every action docs/themes.md lists for a theme\'s own buttons is an action web/ runs', () => {
  const docs = readFileSync(join(ROOT, 'docs', 'themes.md'), 'utf8');
  const at = docs.indexOf('**The actions a button of yours can name**');
  const rows = at < 0 ? -1 : docs.indexOf('|---|', at), table = rows < 0 ? '' : docs.slice(rows, docs.indexOf('\n\n', rows));
  const listed = [...table.matchAll(/^\| ((?:`[a-z][a-z0-9-]*`(?:, )?)+) \|/gm)].flatMap((m) => [...m[1].matchAll(/`([^`]+)`/g)].map((x) => x[1]));
  assert.ok(listed.length > 20, 'docs/themes.md section 15 lists the actions');
  assert.deepEqual(listed.filter((a) => !registered.has(a)), []);
});
