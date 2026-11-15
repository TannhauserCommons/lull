// What each route's start ships, in bytes, summed from the built web files with no browser, hub or network (beyond the
// `npm ci` the build itself needs): `node scripts/budgets/measure.mjs [tree]`, JSON on stdout.
//
// A tree's web/ goes through scripts/web-dist/build.mjs, exactly as the image's build does it, and each file a first
// load of the route fetches is counted at the bytes the hub sends for it (crates/server/src/web.rs): the pre-compressed
// brotli file, else the gzip one, else the file as it is, since every browser lull supports accepts brotli. Response
// headers are not counted: they are the same few hundred bytes for every file (the CSP is most of them), so a new file
// costs its body plus that.
//
// Which files a first load fetches, in order of what decides it:
//   1. index.html: its stylesheets, its `@import`s and its scripts. Its inline scripts are not read (the desktop app's
//      document.write is not the browser's start).
//   2. The default design's own files (designs/<id>.js and designs/<id>/theme.css; the id is designs.js's fallback (the minified build has renamed D)),
//      what a first visit boots.
//   3. What the stylesheets pull in: their `@import`s and the fonts their @font-face rules name. Any other url() in a
//      stylesheet is loaded only when a rule matches, which a build can't tell, so each must be listed under
//      `notAtStart` in the budgets file with its reason; one that isn't fails, so a new image can't slip in unmeasured.
//   4. The route's own `loads` in the budgets file (the code its `need` fetches when it opens), with their stylesheets'
//      pulls.
// Left out, as shipped.mjs leaves them out: what the browser fetches after the page has loaded (the page's icons and its
// manifest), and the hub's own answers (the library's covers, its API). Fonts are counted when a start stylesheet
// declares them, whether or not the first paint uses them: a build can't tell, and a font nobody uses is waste.
import { execFileSync, execSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, posix, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
export const BUDGETS_FILE = 'scripts/budgets/shipped-bytes.json';
// The encodings the hub sends, in its order (web.rs ENCODINGS), with the suffix of the file the build wrote.
const ENCODINGS = [['br', '.br'], ['gz', '.gz']];
// <link rel> values that fetch something after load (or never): left out. Any other rel that fetches is an error.
const AFTER_LOAD = new Set(['icon', 'apple-touch-icon', 'manifest']);
const NO_FETCH = new Set(['canonical', 'alternate', 'author', 'license', 'search']);
// Elements that fetch a file from index.html itself. None is there today; one that appears needs a decision here.
const FETCHING_TAGS = /<(img|source|video|audio|iframe|embed|object|track|input\s[^>]*type=["']?image)\b/i;

const attr = (tag, name) => new RegExp(`\\s${name}=(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`, 'i').exec(tag)?.slice(1).find((v) => v !== undefined);

// What index.html loads by itself: { stylesheets, scripts } as paths from the web root.
export function indexFiles(html) {
  const inline = /<script(?![^>]*\ssrc=)[^>]*>[\s\S]*?<\/script>/gi;
  const page = html.replace(inline, '');
  if (FETCHING_TAGS.test(page)) throw new Error('index.html has an element that fetches a file (img, source, video, ...): decide how the start counts it in scripts/budgets/measure.mjs');
  const stylesheets = [], scripts = [];
  for (const [tag] of page.matchAll(/<link\b[^>]*>/gi)) {
    const rels = (attr(tag, 'rel') ?? '').toLowerCase().split(/\s+/);
    const href = attr(tag, 'href');
    if (rels.includes('stylesheet')) { if (href) stylesheets.push(href); continue; }
    if (rels.some((r) => AFTER_LOAD.has(r) || NO_FETCH.has(r))) continue;
    throw new Error(`index.html has a <link rel="${rels.join(' ')}">: decide how the start counts it in scripts/budgets/measure.mjs`);
  }
  for (const [, css] of page.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)) stylesheets.push(...importsOf(css));
  for (const [tag] of page.matchAll(/<script\b[^>]*>/gi)) { const src = attr(tag, 'src'); if (src) scripts.push(src); }
  return { stylesheets, scripts };
}

const importsOf = (css) => [...css.matchAll(/@import\s*(?:url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)|"([^"]*)"|'([^']*)')/gi)]
  .map((m) => m.slice(1).find((v) => v !== undefined));

// What a stylesheet pulls in, as paths relative to the stylesheet: { imports, fonts, others } (others: every other url()).
export function cssRefs(css) {
  const fonts = [], others = [];
  const rest = css.replace(/@font-face\s*\{[^}]*\}/gi, (block) => { fonts.push(...urlsOf(block)); return ''; });
  const imports = importsOf(rest);
  for (const url of urlsOf(rest.replace(/@import\s*url\([^)]*\)/gi, ''))) others.push(url);
  return { imports, fonts, others };
}

const urlsOf = (css) => [...css.matchAll(/url\(\s*(?:"([^"]*)"|'([^']*)'|([^)\s]*))\s*\)/gi)]
  .map((m) => m.slice(1).find((v) => v !== undefined)).filter((u) => u && !/^(data:|#|[a-z][a-z0-9+.-]*:|\/\/)/i.test(u));

// The built file a browser gets for `path`, and its bytes on the wire.
export function wireFile(dir, path) {
  for (const [encoding, suffix] of ENCODINGS) {
    const file = join(dir, path + suffix);
    if (existsSync(file)) return { path, encoding, bytes: statSync(file).size };
  }
  const file = join(dir, path);
  if (!existsSync(file)) throw new Error(`${path} is referenced at start but the build made no such file`);
  return { path, encoding: 'identity', bytes: statSync(file).size };
}

// The design a first visit boots: designs.js's fallback (the minified build has renamed D).
export function defaultDesign(dir) {
  const id = /\.fallback\s*=\s*['"]([a-z0-9-]+)['"]/.exec(readFileSync(join(dir, 'designs/designs.js'), 'utf8'))?.[1];
  if (!id) throw new Error('designs/designs.js no longer sets D.fallback to a design id: update scripts/budgets/measure.mjs');
  return id;
}

// The files route `route` fetches at start, from the build in `dir`: sorted by path, once each, with the bytes of each.
// `model` is the budgets file's { notAtStart, routes }. `skipped` is the notAtStart paths this route's stylesheets name.
export function startOf(dir, route, model) {
  const spec = model.routes?.[route];
  if (!spec) throw new Error(`no route ${route} in the budgets file: ${Object.keys(model.routes ?? {}).join(', ')}`);
  const design = defaultDesign(dir);
  const { stylesheets, scripts } = indexFiles(readFileSync(join(dir, 'index.html'), 'utf8'));
  const loads = Object.keys(spec.loads ?? {});
  const seen = new Set(['index.html', `designs/${design}.js`, ...scripts, ...loads.filter((p) => !p.endsWith('.css'))]);
  const skipped = new Set(), visited = new Set();
  const sheets = [...stylesheets, `designs/${design}/theme.css`, ...loads.filter((p) => p.endsWith('.css'))];
  while (sheets.length) {
    const sheet = posix.normalize(sheets.shift());
    if (visited.has(sheet)) continue;
    visited.add(sheet);
    seen.add(sheet);
    const { imports, fonts, others } = cssRefs(readFileSync(join(dir, sheet), 'utf8'));
    const from = (url) => posix.normalize(posix.join(posix.dirname(sheet), url.split(/[?#]/)[0]));
    sheets.push(...imports.map(from));
    for (const font of fonts) seen.add(from(font));
    for (const other of others) {
      const path = from(other);
      if (!Object.hasOwn(model.notAtStart ?? {}, path)) {
        throw new Error(`${sheet} loads ${path}: a build can't tell whether a first load fetches it. List it under notAtStart in ${BUDGETS_FILE} with its reason, or leave it out of the start's stylesheets`);
      }
      skipped.add(path);
    }
  }
  const files = [...seen].sort().map((path) => wireFile(dir, path));
  return { files, bytes: files.reduce((sum, f) => sum + f.bytes, 0), skipped };
}

// Builds `tree`'s web/ as the image does, into a folder that is removed again, and measures every route in `model`
// (default: the tree's own budgets file). Returns { route: { bytes, files: [{ path, encoding, bytes }] } }.
export function measureTree(tree, model = readModel(tree)) {
  const out = mkdtempSync(join(tmpdir(), 'lull-budgets-'));
  try {
    build(tree, out);
    const routes = {}, skipped = new Set();
    for (const route of Object.keys(model.routes)) {
      const { files, bytes, skipped: s } = startOf(out, route, model);
      routes[route] = { bytes, files };
      for (const path of s) skipped.add(path);
    }
    for (const path of Object.keys(model.notAtStart ?? {})) {
      if (!skipped.has(path)) throw new Error(`notAtStart lists ${path}, but no start stylesheet names it any more: remove it from ${BUDGETS_FILE}`);
    }
    return routes;
  } finally {
    rmSync(out, { recursive: true, force: true });
  }
}

export function readModel(tree) {
  return JSON.parse(readFileSync(join(tree, BUDGETS_FILE), 'utf8'));
}

// scripts/web-dist/build.mjs on the tree's own web/, after the `npm ci` its `web:dist` task runs (a no-op when done).
function build(tree, out) {
  const dist = join(tree, 'scripts/web-dist');
  if (!existsSync(join(dist, 'node_modules'))) {
    // One command string through the shell: on Windows npm is a .cmd file that execFile won't start.
    execSync(`npm ci --prefix "${dist}" --ignore-scripts --no-audit --no-fund`, { stdio: ['ignore', 'ignore', 'inherit'] });
  }
  execFileSync(process.execPath, [join(dist, 'build.mjs'), join(tree, 'web'), out], { stdio: ['ignore', 'ignore', 'inherit'] });
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const tree = resolve(process.argv[2] ?? join(here, '../..'));
  const routes = measureTree(tree);
  console.log(JSON.stringify(routes, null, 2));
}
