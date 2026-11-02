// Production web assets: `node scripts/web-dist/build.mjs <web dir> <out dir>`.
// Minifies each CSS file (lightningcss) and JS file (esbuild) in place of the original, never
// bundled: index.html loads scripts in a fixed order and designs swap stylesheets by file name.
// Then writes a .br and a .gz next to every text file that compresses, for web.rs to serve by
// Accept-Encoding. <out dir> may be <web dir> itself (a CI checkout about to build the app).
// Prints the sizes before and after. Used by the Dockerfile; see docs/context/quality.md.

import { mkdir, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, extname, join, relative, resolve } from 'node:path';
import { brotliCompressSync, constants, gzipSync } from 'node:zlib';
import { transform as minifyJs } from 'esbuild';
import { transform as minifyCss } from 'lightningcss';

// Only what the hub serves: the rest of web/ is Webamp's npm install.
const SKIP = new Set(['node_modules', 'package.json', 'package-lock.json']);
// Text that compresses; images and fonts are compressed already.
const COMPRESS = new Set(['.html', '.css', '.js', '.mjs', '.svg', '.json', '.webmanifest', '.txt', '.ico']);
// A variant must save at least this much to be worth a second lookup and a Vary.
const MIN_SAVING = 0.05;

const [src, out] = process.argv.slice(2).map((p) => p && resolve(p));
if (!src || !out) {
  console.error('usage: node build.mjs <web dir> <out dir>');
  process.exit(2);
}

// Bytes per kind of file (css, js, html, other) and stage (source, minified): raw, gzip, brotli.
const totals = {};
for (const path of await files(src)) {
  const rel = relative(src, path);
  const before = await readFile(path);
  const body = await minify(rel, before);
  const target = join(out, rel);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, body);
  const kind = ['.css', '.js', '.html'].includes(extname(rel)) ? extname(rel).slice(1) : 'other';
  const row = (totals[kind] ??= { files: 0, source: sizes0(), minified: sizes0() });
  add(row.source, measure(target, before));
  add(row.minified, await precompress(target, body));
  row.files += 1;
}
report(totals);

async function files(dir) {
  const found = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (SKIP.has(entry.name)) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...(await files(path)));
    // A rerun in place must not compress the compressed copies again.
    else if (!/\.(br|gz)$/.test(entry.name)) found.push(path);
  }
  return found;
}

// Already-minified vendor bundles (Webamp) are left as they are.
async function minify(rel, bytes) {
  const ext = extname(rel);
  if (rel.endsWith('.min.js')) return bytes;
  if (ext === '.css') {
    // errorRecovery stays off: CSS lightningcss can't parse fails the build instead of shipping changed.
    return Buffer.from(minifyCss({ filename: rel, code: bytes, minify: true }).code);
  }
  if (ext === '.js') {
    // No `format`: the files stay classic scripts, so esbuild keeps their top-level names (the
    // globals other scripts read) and only renames what is local. No `target`: nothing is lowered.
    const { code } = await minifyJs(bytes.toString('utf8'), { loader: 'js', minify: true, sourcefile: rel });
    return Buffer.from(code);
  }
  return bytes;
}

// Writes the variants worth keeping; returns the bytes a browser gets for each encoding.
async function precompress(target, body) {
  for (const enc of ['br', 'gz']) await rm(`${target}.${enc}`, { force: true });
  const { variants, sizes } = compress(target, body);
  for (const [enc, bytes] of Object.entries(variants)) await writeFile(`${target}.${enc}`, bytes);
  return sizes;
}

function measure(target, body) {
  return compress(target, body).sizes;
}

// The variants that save at least MIN_SAVING, and the size served for each encoding.
function compress(target, body) {
  const sizes = { raw: body.length, gz: body.length, br: body.length };
  const variants = {};
  if (!COMPRESS.has(extname(target))) return { variants, sizes };
  const all = {
    br: brotliCompressSync(body, {
      params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_SIZE_HINT]: body.length },
    }),
    gz: gzipSync(body, { level: 9 }),
  };
  for (const [enc, bytes] of Object.entries(all)) {
    if (bytes.length > body.length * (1 - MIN_SAVING)) continue;
    variants[enc] = bytes;
    sizes[enc] = bytes.length;
  }
  return { variants, sizes };
}

function sizes0() {
  return { raw: 0, gz: 0, br: 0 };
}

function add(into, sizes) {
  for (const k of Object.keys(into)) into[k] += sizes[k];
}

function report(totals) {
  const kb = (n) => `${(n / 1024).toFixed(1).padStart(8)} KiB`;
  const all = { files: 0, source: sizes0(), minified: sizes0() };
  console.log('web assets          files          raw         gzip       brotli');
  for (const [kind, row] of [...Object.entries(totals), ['all', all]]) {
    for (const stage of ['source', 'minified']) {
      const s = row[stage];
      console.log(`  ${kind.padEnd(6)} ${stage.padEnd(9)} ${String(row.files).padStart(4)} ${kb(s.raw)} ${kb(s.gz)} ${kb(s.br)}`);
      if (kind !== 'all') add(all[stage], s);
    }
    if (kind !== 'all') all.files += row.files;
  }
}
