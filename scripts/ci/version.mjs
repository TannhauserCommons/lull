// lull's version, for the shared CI skeleton's `version` and `bump` tasks (docs/context/quality.md, Where the version lives).
//   node scripts/ci/version.mjs [print]   the version of Cargo.toml's [workspace.package], as the last line of stdout
//   node scripts/ci/version.mjs check     fail, naming the file, when a file that holds the version disagrees with it
//   node scripts/ci/version.mjs bump X.Y.Z  write X.Y.Z into every file that holds it (no cargo, no network)
// The holders are listed once, in HOLDERS and the helpers around it: the two manifests, Android's versionCode, the `lull-*`
// packages of both lock files, and the F-Droid changelog the code names. Lock entries are edited by package name, never by
// version string: app/Cargo.lock has a dependency (`document-features`) that happens to share lull's version number.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const MANIFEST = 'Cargo.toml';
const APP_MANIFEST = 'app/Cargo.toml';
const ANDROID_CONF = 'app/tauri.android.conf.json';
const LOCKS = ['Cargo.lock', 'app/Cargo.lock'];
const CHANGELOGS = 'fastlane/metadata/android/en-US/changelogs';
/** What scripts/release.sh writes when the commits since the last release have no feat or fix to list. */
const CHANGELOG_STUB = 'Smaller fixes.\n';
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
/** Android's versionCode is a signed 32-bit integer, and Google Play caps it at this. */
const MAX_CODE = 2_100_000_000;

/** The files that hold the version (not the changelog, which is named by the code). */
export const HOLDER_FILES = [MANIFEST, APP_MANIFEST, ANDROID_CONF, ...LOCKS];

/** Android's versionCode, X*1000000 + Y*1000 + Z (Tauri's own formula; scripts/release.sh), or throws why a version has none. */
export function versionCode(version) {
  const match = VERSION.exec(version);
  if (!match) throw new Error(`${JSON.stringify(version)} is not X.Y.Z (digits only, no leading zeros)`);
  const [x, y, z] = match.slice(1).map(Number);
  if (y >= 1000 || z >= 1000) throw new Error(`minor and patch must stay below 1000 (Android's versionCode), got ${version}`);
  const code = x * 1_000_000 + y * 1000 + z;
  if (code > MAX_CODE) throw new Error(`${version} gives versionCode ${code}, above Android's ${MAX_CODE}`);
  return code;
}

/** Whether `a` is a newer X.Y.Z than `b`. */
function newer(a, b) {
  const [pa, pb] = [a, b].map((v) => v.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] > pb[i];
  return false;
}

/** The text as lines that keep their own ending, so a CRLF checkout stays CRLF and an LF one LF. */
const linesOf = (text) => text.match(/[^\n]*\n|[^\n]+/g) ?? [];

/** The [table] a line opens, or null for any other line. */
function header(line) {
  const match = /^\s*\[\[?([^\[\]]+)\]\]?\s*(#.*)?$/.exec(line);
  return match ? match[1].trim() : null;
}

const VERSION_LINE = /^(\s*version\s*=\s*")([^"]*)(".*?)(\r?\n?)$/s;

/** Where the first `version = "..."` of `[section]` is: the line's index, or -1. */
function versionLineIn(lines, section) {
  let current = null;
  for (let i = 0; i < lines.length; i++) {
    const name = header(lines[i]);
    if (name !== null) current = name;
    else if (current === section && VERSION_LINE.test(lines[i])) return i;
  }
  return -1;
}

const setVersionLine = (line, version) => line.replace(VERSION_LINE, (_, head, _old, tail, eol) => `${head}${version}${tail}${eol}`);
const versionOfLine = (line) => VERSION_LINE.exec(line)[2];

/** The version a manifest's [section] holds, or null. */
function manifestVersion(text, section) {
  const lines = linesOf(text);
  const at = versionLineIn(lines, section);
  return at < 0 ? null : versionOfLine(lines[at]);
}

function setManifestVersion(file, text, section, version) {
  const lines = linesOf(text);
  const at = versionLineIn(lines, section);
  if (at < 0) throw new Error(`${file}: no version line in [${section}]`);
  lines[at] = setVersionLine(lines[at], version);
  return lines.join('');
}

/** The lull-owned packages of a lock file: the ones named `lull-*` that have no `source` (they are paths, not crates.io). */
function lockEntries(text) {
  const lines = linesOf(text);
  const entries = [];
  let entry = null;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\[\[package\]\]\s*$/.test(line)) {
      entry = { name: null, versionAt: -1, source: false };
      entries.push(entry);
    } else if (header(line) !== null) entry = null;
    else if (entry) {
      const name = /^name = "([^"]*)"/.exec(line);
      if (name) entry.name = name[1];
      else if (/^version = "/.test(line)) entry.versionAt = i;
      else if (/^source = /.test(line)) entry.source = true;
    }
  }
  // `source` can come after `version`, so a package is only known to be a path one once its whole block was read.
  return { lines, entries: entries.filter((e) => e.name !== null && e.versionAt >= 0 && /^lull-/.test(e.name) && !e.source) };
}

/** The `lull-*` path packages of a lock file as [name, version, line index]. */
function ownLocks(text) {
  const { lines, entries } = lockEntries(text);
  return { lines, own: entries.map((e) => [e.name, versionOfLine(lines[e.versionAt]), e.versionAt]) };
}

function setLockVersions(text, version) {
  const { lines, own } = ownLocks(text);
  for (const [, , at] of own) lines[at] = setVersionLine(lines[at], version);
  return lines.join('');
}

const readFile = (root, file) => readFileSync(join(root, file), 'utf8');

/** The text of every holder file under `root`. */
function readHolders(root) {
  return Object.fromEntries(HOLDER_FILES.map((file) => [file, readFile(root, file)]));
}

/** The version `Cargo.toml` carries, from the files' text. */
function carried(files) {
  const version = manifestVersion(files[MANIFEST], 'workspace.package');
  if (!version) throw new Error(`${MANIFEST}: no version in [workspace.package]`);
  return version;
}

/** The versionCode in app/tauri.android.conf.json, or null. */
function androidCode(text) {
  const code = JSON.parse(text)?.bundle?.android?.versionCode;
  return Number.isInteger(code) ? code : null;
}

/** Every way a holder disagrees with Cargo.toml, each naming its file; empty when they all agree. */
export function problemsIn(files) {
  const problems = [];
  let version;
  try {
    version = carried(files);
  } catch (error) {
    return [error.message];
  }
  let code = null;
  try {
    code = versionCode(version);
  } catch (error) {
    problems.push(`${MANIFEST}: ${error.message}`);
  }

  const app = manifestVersion(files[APP_MANIFEST], 'package');
  if (app !== version) problems.push(`${APP_MANIFEST}: version is ${app ?? 'missing'}, ${MANIFEST} has ${version}`);

  const have = androidCode(files[ANDROID_CONF]);
  if (have === null) problems.push(`${ANDROID_CONF}: no bundle.android.versionCode`);
  else if (code !== null && have !== code) problems.push(`${ANDROID_CONF}: versionCode is ${have}, ${version} gives ${code}`);

  for (const lock of LOCKS) {
    const { own } = ownLocks(files[lock]);
    if (!own.length) problems.push(`${lock}: no lull-* packages`);
    for (const [name, locked] of own) {
      if (locked !== version) problems.push(`${lock}: ${name} is ${locked}, ${MANIFEST} has ${version}`);
    }
  }
  // The app's own package is the one entry the app's lock must have besides the hub's crates.
  if (!ownLocks(files['app/Cargo.lock']).own.some(([name]) => name === 'lull-app')) {
    problems.push('app/Cargo.lock: no lull-app package');
  }
  return problems;
}

/** `problemsIn` for the tree at `root`. */
export const problems = (root) => problemsIn(readHolders(root));

/** The version the tree at `root` carries. */
export const readVersion = (root) => carried(readHolders(root));

function changelogPath(code) {
  return `${CHANGELOGS}/${code}.txt`;
}

/** Stage `file` when `root` is a git work tree: the skeleton commits a bump with `git commit -am`, which skips new files. */
function stage(root, file) {
  let inside;
  try {
    inside = execFileSync('git', ['rev-parse', '--is-inside-work-tree'], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return false; // not a repository (or no git): there is nothing to commit into
  }
  if (inside !== 'true') return false;
  execFileSync('git', ['add', '--', file], { cwd: root, stdio: 'inherit' });
  return true;
}

/**
 * Write `version` into every holder under `root`. Refuses anything that is not X.Y.Z, a version that is not newer than the
 * one Cargo.toml carries, and a versionCode that does not rise; writes nothing unless every file is computed and agrees.
 * `stage: true` also `git add`s the changelog stub. Returns the files it wrote.
 */
export function bump(root, version, { stage: staging = false } = {}) {
  const code = versionCode(version);
  const files = readHolders(root);
  const current = carried(files);
  if (!newer(version, current)) throw new Error(`${version} is not newer than ${current}`);
  const oldCode = androidCode(files[ANDROID_CONF]);
  if (oldCode === null) throw new Error(`${ANDROID_CONF}: no bundle.android.versionCode`);
  if (code <= oldCode) throw new Error(`versionCode ${code} does not rise above ${oldCode} (${ANDROID_CONF})`);

  const next = { ...files };
  next[MANIFEST] = setManifestVersion(MANIFEST, files[MANIFEST], 'workspace.package', version);
  next[APP_MANIFEST] = setManifestVersion(APP_MANIFEST, files[APP_MANIFEST], 'package', version);
  const codeField = /("versionCode"\s*:\s*)\d+/g;
  if ((files[ANDROID_CONF].match(codeField) ?? []).length !== 1) throw new Error(`${ANDROID_CONF}: expected one versionCode`);
  next[ANDROID_CONF] = files[ANDROID_CONF].replace(codeField, `$1${code}`);
  for (const lock of LOCKS) next[lock] = setLockVersions(files[lock], version);

  const left = problemsIn(next);
  if (left.length) throw new Error(`the bump would leave the files disagreeing:\n  ${left.join('\n  ')}`);

  const written = HOLDER_FILES.filter((file) => next[file] !== files[file]);
  for (const file of written) writeFileSync(join(root, file), next[file]);
  const stub = changelogPath(code);
  if (!existsSync(join(root, stub))) {
    mkdirSync(join(root, dirname(stub)), { recursive: true });
    writeFileSync(join(root, stub), CHANGELOG_STUB);
    written.push(stub);
  }
  if (staging) stage(root, stub);
  return written;
}

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..');

function main(args) {
  const [command = 'print', ...rest] = args;
  try {
    if (command === 'print' && !rest.length) {
      console.log(readVersion(ROOT));
    } else if (command === 'check' && !rest.length) {
      const found = problems(ROOT);
      if (found.length) {
        console.error(`version: the files disagree about the version:\n  ${found.join('\n  ')}`);
        return 1;
      }
      console.log(`version ${readVersion(ROOT)} agrees in ${HOLDER_FILES.length} files`);
    } else if (command === 'bump' && rest.length === 1) {
      const version = rest[0];
      const written = bump(ROOT, version, { stage: true });
      console.log(`bumped to ${version} (versionCode ${versionCode(version)}): ${written.join(', ')}`);
      console.log(version);
    } else {
      console.error('usage: node scripts/ci/version.mjs [print | check | bump X.Y.Z]');
      return 2;
    }
  } catch (error) {
    console.error(`version: ${error.message}`);
    return 1;
  }
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) process.exitCode = main(process.argv.slice(2));
