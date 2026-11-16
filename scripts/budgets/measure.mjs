// What each route's start ships, in bytes: `node scripts/budgets/measure.mjs [tree]`, JSON on stdout.
//
// A tree's web/ goes through its own scripts/web-dist/build.mjs, as the image's build does it. A real browser lists the files
// a first load of each route fetches from that build (list.mjs: no scanner of HTML or CSS guesses what the app's scripts
// will load), and each listed file counts at the bytes the hub sends for it, read from the built tree in web.rs's order: the
// pre-compressed brotli file, else the gzip one, else the file as it is, since every browser lull supports accepts brotli.
// Response headers are not counted: they are the same few hundred bytes for every file (the CSP is most of them), so a new
// file costs its body plus that. The hub's own answers (the library's covers, its API) are not files of the tree.
import { execFileSync, execSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { listRoutes } from './list.mjs';

const here = dirname(fileURLToPath(import.meta.url));
export const BUDGETS_FILE = 'scripts/budgets/shipped-bytes.json';
// The encodings the hub sends, in its order (web.rs ENCODINGS), with the suffix of the file the build wrote.
const ENCODINGS = [['br', '.br'], ['gz', '.gz']];

// The built file a browser gets for `path`, and its bytes on the wire.
export function wireFile(dir, path) {
  for (const [encoding, suffix] of ENCODINGS) {
    const file = join(dir, path + suffix);
    if (existsSync(file)) return { path, encoding, bytes: statSync(file).size };
  }
  const file = join(dir, path);
  if (!existsSync(file)) throw new Error(`${path} was fetched but the build made no such file`);
  return { path, encoding: 'identity', bytes: statSync(file).size };
}

// `paths` sized from the build in `dir`: { files, bytes }.
export function sizeOf(dir, paths) {
  const files = [...paths].sort().map((path) => wireFile(dir, path));
  return { files, bytes: files.reduce((sum, f) => sum + f.bytes, 0) };
}

export function readLimits(tree) {
  return JSON.parse(readFileSync(join(tree, BUDGETS_FILE), 'utf8'));
}

// Builds `tree`'s web/ into a folder that is removed again, lists every route in a browser and sizes what it fetched:
// { route: { bytes, files: [{ path, encoding, bytes }] } }. `chromium` is Playwright's, from the tree that runs the check.
export async function measureTree(tree, routes, chromium) {
  const dist = mkdtempSync(join(tmpdir(), 'lull-budgets-'));
  try {
    build(tree, dist);
    const listed = await listRoutes(chromium, tree, dist, routes);
    return Object.fromEntries(routes.map((route) => [route, sizeOf(dist, listed[route])]));
  } finally {
    rmSync(dist, { recursive: true, force: true });
  }
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

// Playwright from the checkout that runs the check (`mise run ui:setup`'s npm ci): a merge-base's tree has none.
export async function loadChromium(root) {
  const file = join(root, 'tests/ui/node_modules/@playwright/test/index.mjs');
  if (!existsSync(file)) throw new Error('Playwright is not installed: run `sh tests/ui/setup.sh chromium` (the budgets task does)');
  return (await import(pathToFileURL(file).href)).chromium;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const root = resolve(here, '../..');
  const tree = resolve(process.argv[2] ?? root);
  const routes = Object.keys(readLimits(tree).routes);
  console.log(JSON.stringify(await measureTree(tree, routes, await loadChromium(root)), null, 2));
}
