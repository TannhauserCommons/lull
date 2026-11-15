// lull's version, for the shared CI skeleton's `version` and `bump` tasks (docs/context/quality.md, Where the version lives).
//   node scripts/ci/version.mjs [print]   the version of Cargo.toml's [workspace.package], as the last line of stdout
//   node scripts/ci/version.mjs check     fail, naming the file, when a file that holds the version disagrees with it
//   node scripts/ci/version.mjs bump X.Y.Z  write X.Y.Z into every file that holds it (no cargo, no network)
// The holders: the two manifests and every workspace member's manifest (which takes the workspace version or equals it),
// Android's versionCode, and the packages of both lock files that are lull's own (every workspace member, the app, and the
// members the app reaches). Lock entries are edited by package name, never by version string: app/Cargo.lock has a
// dependency (`document-features`) that happens to share lull's version number. The F-Droid changelog is written, not checked.
// A bump prepares everything that can fail (temp files, the changelog stub, staging it) before the first holder changes, and
// rolls back if a holder cannot be replaced; a rerun at a version the tree already partly carries completes what is missing.
import { existsSync, mkdirSync, readFileSync, realpathSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const MANIFEST = 'Cargo.toml';
const APP_MANIFEST = 'app/Cargo.toml';
const ANDROID_CONF = 'app/tauri.android.conf.json';
const LOCKS = ['Cargo.lock', 'app/Cargo.lock'];
/** Each Cargo workspace: its manifest, the folder it sits in, its lock file. The app is a workspace of its own. */
const WORKSPACES = [
  [MANIFEST, '', 'Cargo.lock'],
  [APP_MANIFEST, 'app/', 'app/Cargo.lock'],
];
const CHANGELOGS = 'fastlane/metadata/android/en-US/changelogs';
/** What scripts/release.sh writes when the commits since the last release have no feat or fix to list. */
const CHANGELOG_STUB = 'Smaller fixes.\n';
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
/** Android's versionCode is a signed 32-bit integer, and Google Play caps it at this. */
const MAX_CODE = 2_100_000_000;

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

/** -1, 0 or 1 as the X.Y.Z `a` is older than, the same as or newer than `b` (both already valid). */
function compare(a, b) {
  const [pa, pb] = [a, b].map((v) => v.split('.').map(Number));
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] > pb[i] ? 1 : -1;
  return 0;
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

/** The `members = [...]` paths of a manifest's [workspace] (none when it has no such list). */
function workspaceMembers(text) {
  const lines = linesOf(text);
  let current = null;
  for (let i = 0; i < lines.length; i++) {
    const name = header(lines[i]);
    if (name !== null) current = name;
    else if (current === 'workspace' && /^\s*members\s*=/.test(lines[i])) {
      let list = '';
      for (let j = i; j < lines.length; j++) {
        list += lines[j].replace(/#.*$/, '');
        if (list.includes(']')) break;
      }
      return [...list.matchAll(/"([^"]*)"/g)].map((m) => m[1]);
    }
  }
  return [];
}

const INHERITS = /^\s*version\s*\.\s*workspace\s*=\s*true\b|^\s*version\s*=\s*\{[^}]*\bworkspace\s*=\s*true/;

/** A manifest's [package]: its name (null when it has none), its own version and whether it takes the workspace's. */
function packageOf(text) {
  const found = { name: null, version: null, inherits: false };
  let current = null;
  for (const line of linesOf(text)) {
    const name = header(line);
    if (name !== null) current = name;
    else if (current === 'package') {
      const own = /^\s*name\s*=\s*"([^"]*)"/.exec(line);
      if (own) found.name ??= own[1];
      else if (VERSION_LINE.test(line)) found.version ??= versionOfLine(line);
      else if (INHERITS.test(line)) found.inherits = true;
    }
  }
  return found;
}

/** The packages of one workspace that lull versions: its listed members, then its own root package if the manifest has one. */
function workspacePackages(files, [manifest, dir]) {
  const members = workspaceMembers(files[manifest]).filter((m) => !m.includes('*')).map((m) => `${dir}${m}/Cargo.toml`);
  const own = packageOf(files[manifest]).name === null ? [] : [manifest];
  return [...members, ...own];
}

const escaped = (name) => name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

/**
 * The package names each lock file must hold: the workspace's own packages, and for the app the root members it reaches
 * (a member is reached when a manifest of the closure names it as a dependency; the lock lists them whatever the target).
 */
function expectedLocks(files) {
  const names = ([manifest, dir]) => workspacePackages(files, [manifest, dir]).filter((f) => files[f] !== undefined).map((f) => [packageOf(files[f]).name, f]);
  const [root, app] = WORKSPACES.map((w) => names(w));
  const reached = new Map(app);
  const queue = [...app.map(([, file]) => file)];
  const known = new Map(root);
  while (queue.length) {
    const text = files[queue.pop()];
    for (const [name, file] of known) {
      if (name && !reached.has(name) && new RegExp(`^\\s*${escaped(name)}\\s*[.=]`, 'm').test(text)) {
        reached.set(name, file);
        queue.push(file);
      }
    }
  }
  return { 'Cargo.lock': new Set(root.map(([n]) => n).filter(Boolean)), 'app/Cargo.lock': new Set([...reached.keys()].filter(Boolean)) };
}

/** A lock file's packages that lull owns: the wanted ones and any other `lull-*`, none of which has a `source` (they are paths). */
function lockEntries(text, wanted) {
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
  const own = entries.filter((e) => e.name !== null && e.versionAt >= 0 && !e.source && (wanted.has(e.name) || /^lull-/.test(e.name)));
  return { lines, own: own.map((e) => [e.name, versionOfLine(lines[e.versionAt]), e.versionAt]) };
}

function setLockVersions(text, wanted, version) {
  const { lines, own } = lockEntries(text, wanted);
  for (const [, , at] of own) lines[at] = setVersionLine(lines[at], version);
  return lines.join('');
}

const readFile = (root, file) => readFileSync(join(root, file), 'utf8');

/** The text of every file that holds the version under `root`, by its path from `root`. */
function readTree(root) {
  const files = {};
  for (const file of [MANIFEST, APP_MANIFEST, ANDROID_CONF, ...LOCKS]) files[file] = readFile(root, file);
  for (const workspace of WORKSPACES) {
    for (const file of workspacePackages(files, workspace)) if (files[file] === undefined && existsSync(join(root, file))) files[file] = readFile(root, file);
  }
  return files;
}

/** The files that hold the version under `root` (what a bump may change and `check` reads). */
export const trackedFiles = (root) => Object.keys(readTree(root));

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

  for (const workspace of WORKSPACES) {
    const [manifest] = workspace;
    const shared = manifestVersion(files[manifest], 'workspace.package');
    for (const file of workspacePackages(files, workspace)) {
      if (files[file] === undefined) {
        problems.push(`${file}: missing, but ${manifest} lists it as a member`);
        continue;
      }
      const pkg = packageOf(files[file]);
      if (pkg.name === null) problems.push(`${file}: no [package] name`);
      else if (pkg.inherits && shared !== version) problems.push(`${file}: takes the workspace version, which ${manifest} has as ${shared ?? 'nothing'}, not ${version}`);
      else if (!pkg.inherits && pkg.version !== version) problems.push(`${file}: version is ${pkg.version ?? 'missing'}, ${MANIFEST} has ${version}`);
    }
  }
  if (packageOf(files[APP_MANIFEST]).name === null) problems.push(`${APP_MANIFEST}: no [package]`);

  const have = androidCode(files[ANDROID_CONF]);
  if (have === null) problems.push(`${ANDROID_CONF}: no bundle.android.versionCode`);
  else if (code !== null && have !== code) problems.push(`${ANDROID_CONF}: versionCode is ${have}, ${version} gives ${code}`);

  const expected = expectedLocks(files);
  for (const lock of LOCKS) {
    const { own } = lockEntries(files[lock], expected[lock]);
    for (const [name, locked] of own) {
      if (locked !== version) problems.push(`${lock}: ${name} is ${locked}, ${MANIFEST} has ${version}`);
    }
    const present = new Set(own.map(([name]) => name));
    for (const name of [...expected[lock]].sort()) if (!present.has(name)) problems.push(`${lock}: no entry for ${name}`);
  }
  return problems;
}

/** `problemsIn` for the tree at `root`. */
export const problems = (root) => problemsIn(readTree(root));

/** The version the tree at `root` carries. */
export const readVersion = (root) => carried(readTree(root));

const changelogPath = (code) => `${CHANGELOGS}/${code}.txt`;

/** The new text of every holder for `version`. */
function retarget(files, version, code) {
  const next = { ...files };
  const expected = expectedLocks(files);
  for (const workspace of WORKSPACES) {
    for (const file of workspacePackages(files, workspace)) {
      if (files[file] !== undefined && !packageOf(files[file]).inherits) next[file] = setManifestVersion(file, files[file], 'package', version);
    }
  }
  next[MANIFEST] = setManifestVersion(MANIFEST, next[MANIFEST], 'workspace.package', version);
  if (manifestVersion(files[APP_MANIFEST], 'workspace.package') !== null) {
    next[APP_MANIFEST] = setManifestVersion(APP_MANIFEST, next[APP_MANIFEST], 'workspace.package', version);
  }
  const codeField = /("versionCode"\s*:\s*)\d+/g;
  if ((files[ANDROID_CONF].match(codeField) ?? []).length !== 1) throw new Error(`${ANDROID_CONF}: expected one versionCode`);
  next[ANDROID_CONF] = files[ANDROID_CONF].replace(codeField, `$1${code}`);
  for (const lock of LOCKS) next[lock] = setLockVersions(files[lock], expected[lock], version);
  return next;
}

const canonical = (path) => realpathSync.native(path);

/** `git add` of `file`, only when `root` is the top of a git work tree: the skeleton commits a bump with `git commit -am`, which skips new files. */
function stage(root, file) {
  let top;
  try {
    top = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
  } catch {
    return false; // not a repository (or no git): there is nothing to commit into
  }
  // Inside some other repository (lull unpacked into one): staging there would stage into the wrong tree.
  if (canonical(top) !== canonical(root)) return false;
  execFileSync('git', ['add', '--', file], { cwd: root, stdio: 'inherit' });
  return true;
}

const unstage = (root, file) => execFileSync('git', ['rm', '--cached', '-q', '--ignore-unmatch', '--', file], { cwd: root, stdio: 'ignore' });

/**
 * Write `version` into every holder under `root`. Refuses anything that is not X.Y.Z, a version older than the one Cargo.toml
 * carries (the same version completes what an earlier run left undone, and is a no-op on a finished tree), a versionCode below
 * the file's, and a holder that is read-only. Everything that can fail runs before a holder changes: the new texts go to temp
 * files, the changelog stub is created and staged (`stage: true`), and only then are the temp files renamed over the holders,
 * each restored if a later one cannot be. Returns `{ written, staged }`.
 */
export function bump(root, version, { stage: staging = false } = {}) {
  const code = versionCode(version);
  const files = readTree(root);
  const current = carried(files);
  if (!VERSION.test(current)) throw new Error(`${MANIFEST}: [workspace.package] has version ${JSON.stringify(current)}, which is not X.Y.Z; fix it by hand before bumping`);
  if (compare(version, current) < 0) throw new Error(`${version} is older than ${current}`);
  const oldCode = androidCode(files[ANDROID_CONF]);
  if (oldCode === null) throw new Error(`${ANDROID_CONF}: no bundle.android.versionCode`);
  if (code < oldCode) throw new Error(`versionCode ${code} does not rise above ${oldCode} (${ANDROID_CONF})`);

  const next = retarget(files, version, code);
  const left = problemsIn(next);
  if (left.length) throw new Error(`the bump would leave the files disagreeing:\n  ${left.join('\n  ')}`);
  const changed = Object.keys(next).filter((file) => next[file] !== files[file]);
  for (const file of changed) {
    if ((statSync(join(root, file)).mode & 0o200) === 0) throw new Error(`${file} is read-only`);
  }

  const stub = changelogPath(code);
  const stubPath = join(root, stub);
  const undo = [];
  const written = [];
  let staged = false;
  try {
    const temps = changed.map((file) => {
      const temp = join(root, `${file}.${process.pid}.tmp`);
      writeFileSync(temp, next[file], { flag: 'wx' });
      undo.push(() => rmSync(temp, { force: true }));
      return [file, temp];
    });
    if (!existsSync(stubPath)) {
      const made = mkdirSync(dirname(stubPath), { recursive: true });
      if (made) undo.push(() => rmSync(made, { recursive: true, force: true }));
      writeFileSync(stubPath, CHANGELOG_STUB, { flag: 'wx' });
      undo.push(() => rmSync(stubPath, { force: true }));
      written.push(stub);
    }
    if (staging) {
      staged = stage(root, stub);
      if (staged && written.includes(stub)) undo.push(() => unstage(root, stub));
    }
    for (const [file, temp] of temps) {
      renameSync(temp, join(root, file));
      undo.push(() => writeFileSync(join(root, file), files[file]));
      written.push(file);
    }
  } catch (error) {
    const stuck = [];
    for (const step of undo.reverse()) {
      try {
        step();
      } catch (again) {
        stuck.push(again.message);
      }
    }
    if (stuck.length) error.message += `\n  and could not undo: ${stuck.join('; ')}\n  run the bump again: it completes what is missing`;
    throw error;
  }
  return { written, staged };
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
      console.log(`version ${readVersion(ROOT)} agrees in ${trackedFiles(ROOT).length} files`);
    } else if (command === 'bump' && rest.length === 1) {
      const version = rest[0];
      const { written, staged } = bump(ROOT, version, { stage: true });
      console.log(`bumped to ${version} (versionCode ${versionCode(version)}): ${written.length ? written.join(', ') : 'nothing to change'}`);
      if (!staged) console.log('the changelog stub was not staged: this is not the top of a git work tree');
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
