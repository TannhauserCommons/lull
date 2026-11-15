// scripts/ci/version.mjs: `mise run version` and `mise run bump`, and the check that every file holding lull's version agrees.
// A bump runs on a temp copy of the real holders, so these tests hold the real files to the script's reading of them.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { bump, HOLDER_FILES, problems, readVersion, versionCode } from '../../scripts/ci/version.mjs';

const repo = resolve(import.meta.dirname, '..', '..');
const CHANGELOGS = 'fastlane/metadata/android/en-US/changelogs';

/** A temp tree holding copies of the real holder files; `cleanup` removes it. */
function copyOfRepo(t) {
  const root = mkdtempSync(join(tmpdir(), 'lull-version-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const file of HOLDER_FILES) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    cpSync(join(repo, file), join(root, file));
  }
  return root;
}

/** Every file under `root` with its bytes, for before and after. */
function snapshot(root, dir = root, into = new Map()) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) snapshot(root, path, into);
    else into.set(path.slice(root.length + 1).replaceAll('\\', '/'), readFileSync(path));
  }
  return into;
}

const changedFiles = (before, after) =>
  [...new Set([...before.keys(), ...after.keys()])].filter((file) => !before.get(file)?.equals(after.get(file) ?? Buffer.alloc(1))).sort();

const nextPatch = (version) => {
  const [x, y, z] = version.split('.').map(Number);
  return `${x}.${y}.${z + 1}`;
};

const edit = (root, file, change) => writeFileSync(join(root, file), change(readFileSync(join(root, file), 'utf8')));

test('the files in the repository agree about the version', () => {
  assert.deepEqual(problems(repo), []);
});

test('`version` prints the version of [workspace.package] as its last line', () => {
  const out = execFileSync('node', [join(repo, 'scripts/ci/version.mjs')], { encoding: 'utf8' });
  assert.equal(out.trimEnd().split('\n').at(-1), readVersion(repo));
  assert.match(readVersion(repo), /^\d+\.\d+\.\d+$/);
});

test('versionCode is X*1000000 + Y*1000 + Z, and refuses what Android cannot hold', () => {
  assert.equal(versionCode('0.2.12'), 2012);
  assert.equal(versionCode('1.0.0'), 1_000_000);
  assert.equal(versionCode('1.999.999'), 1_999_999);
  for (const bad of ['1.1000.0', '1.0.1000', '2101.0.0', '1.2', '1.2.3.4', 'v1.2.3', '1.2.3-rc1', '01.2.3', '1.02.3', '1.2.x', '', ' 1.2.3']) {
    assert.throws(() => versionCode(bad), undefined, bad);
  }
});

test('a bump changes exactly the holders and adds the changelog stub', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  const target = nextPatch(version);
  const before = snapshot(root);

  const written = bump(root, target);

  const stub = `${CHANGELOGS}/${versionCode(target)}.txt`;
  const after = snapshot(root);
  assert.deepEqual(changedFiles(before, after), [...HOLDER_FILES, stub].sort());
  assert.deepEqual(written.sort(), [...HOLDER_FILES, stub].sort());
  assert.equal(readFileSync(join(root, stub), 'utf8'), 'Smaller fixes.\n');
  assert.equal(readVersion(root), target);
  assert.deepEqual(problems(root), []);

  // Only version lines moved: swapping the new version back for the old one restores every holder byte for byte.
  const code = versionCode(target);
  for (const file of HOLDER_FILES) {
    const restored = after.get(file).toString().replaceAll(`"${target}"`, `"${version}"`).replaceAll(`"versionCode": ${code}`, `"versionCode": ${versionCode(version)}`);
    assert.equal(restored, before.get(file).toString(), file);
  }
});

test('a lock entry is edited by package name: a dependency that shares lull\'s version number stays', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  // The trap in app/Cargo.lock today: document-features is 0.2.12 too. Make it so whatever version the repository carries,
  // and add a registry crate named like ours and a path package of another name.
  const trap = (lock) =>
    `${lock}
[[package]]
name = "document-features"
version = "${version}"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00"

[[package]]
name = "lull-registry-lookalike"
version = "${version}"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00"

[[package]]
name = "tauri-plugin-lull-player"
version = "${version}"
`;
  for (const lock of ['Cargo.lock', 'app/Cargo.lock']) edit(root, lock, trap);
  assert.deepEqual(problems(root), [], 'the look-alikes are not lull packages, so they do not disagree');

  const target = nextPatch(version);
  bump(root, target);

  for (const lock of ['Cargo.lock', 'app/Cargo.lock']) {
    const text = readFileSync(join(root, lock), 'utf8');
    for (const name of ['document-features', 'lull-registry-lookalike', 'tauri-plugin-lull-player']) {
      assert.match(text, new RegExp(`name = "${name}"\\nversion = "${version.replaceAll('.', '\\.')}"\\n`), `${lock}: ${name} changed`);
    }
    assert.match(text, new RegExp(`name = "lull-core"\\nversion = "${target.replaceAll('.', '\\.')}"\\n`), `${lock}: lull-core did not move`);
  }
  assert.match(readFileSync(join(root, 'app/Cargo.lock'), 'utf8'), new RegExp(`name = "lull-app"\\nversion = "${target.replaceAll('.', '\\.')}"`));
});

test('a bump that is refused writes nothing', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  const [x, y, z] = version.split('.').map(Number);
  const before = snapshot(root);
  const refused = [
    ['not X.Y.Z', `${x}.${y}`],
    ['a v prefix', `v${nextPatch(version)}`],
    ['a prerelease', `${nextPatch(version)}-rc1`],
    ['a leading zero', `${x}.${y}.0${z + 1}`],
    ['a minor of 1000', `${x}.1000.0`],
    ['a patch of 1000', `${x}.${y}.1000`],
    ['the same version', version],
    ['an older patch', `${x}.${y}.${z - 1}`],
    ['an older minor', `${x}.${y - 1}.${z + 5}`],
  ];
  for (const [why, target] of refused) {
    assert.throws(() => bump(root, target), undefined, why);
    assert.deepEqual(changedFiles(before, snapshot(root)), [], `${why} left files changed`);
  }
});

test('a versionCode that would not rise is refused', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  edit(root, 'app/tauri.android.conf.json', (text) => text.replace(/"versionCode": \d+/, '"versionCode": 99999999'));
  const before = snapshot(root);
  assert.throws(() => bump(root, nextPatch(version)), /does not rise above 99999999/);
  assert.deepEqual(changedFiles(before, snapshot(root)), []);
});

test('bumping to the version it already carries is refused, and the tree stays as the first bump left it', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  bump(root, target);
  const before = snapshot(root);
  assert.throws(() => bump(root, target), /is not newer than/);
  assert.deepEqual(changedFiles(before, snapshot(root)), []);
});

test('an existing changelog for the code is kept', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  const stub = join(root, CHANGELOGS, `${versionCode(target)}.txt`);
  mkdirSync(dirname(stub), { recursive: true });
  writeFileSync(stub, '* Written by hand\n');
  const written = bump(root, target);
  assert.equal(readFileSync(stub, 'utf8'), '* Written by hand\n');
  assert.ok(!written.includes(`${CHANGELOGS}/${versionCode(target)}.txt`));
});

test('line endings stay as they are: CRLF files stay CRLF and LF files LF', (t) => {
  const root = copyOfRepo(t);
  const crlf = ['Cargo.toml', 'app/Cargo.lock', 'app/tauri.android.conf.json'];
  for (const file of crlf) edit(root, file, (text) => text.replaceAll('\r\n', '\n').replaceAll('\n', '\r\n'));
  const before = snapshot(root);
  const version = readVersion(root);
  const target = nextPatch(version);

  bump(root, target);

  const after = snapshot(root);
  for (const file of HOLDER_FILES) {
    const text = after.get(file).toString();
    const bare = text.replaceAll('\r\n', '').includes('\n');
    assert.equal(text.includes('\r\n'), crlf.includes(file), `${file} changed its line endings`);
    assert.equal(bare, !crlf.includes(file), `${file} has mixed line endings`);
    // The same number of lines: nothing was added or dropped.
    assert.equal(text.split('\n').length, before.get(file).toString().split('\n').length, file);
  }
  assert.deepEqual(problems(root), []);
});

test('the check names the file of every holder that disagrees', (t) => {
  const version = readVersion(repo);
  const other = nextPatch(version);
  const cases = [
    ['app/Cargo.toml', (text) => text.replace(`version = "${version}"`, `version = "${other}"`), /^app\/Cargo\.toml: /],
    ['app/tauri.android.conf.json', (text) => text.replace(/"versionCode": \d+/, '"versionCode": 1'), /^app\/tauri\.android\.conf\.json: /],
    ['Cargo.lock', (text) => text.replace(/(name = "lull-core"\nversion = ")[^"]*/, `$1${other}`), /^Cargo\.lock: lull-core is /],
    ['app/Cargo.lock', (text) => text.replace(/(name = "lull-server"\nversion = ")[^"]*/, `$1${other}`), /^app\/Cargo\.lock: lull-server is /],
    ['app/Cargo.lock (lull-app)', (text) => text.replace(/(name = "lull-app"\nversion = ")[^"]*/, `$1${other}`), /^app\/Cargo\.lock: lull-app is /],
    ['app/Cargo.lock (no lull-app)', (text) => text.replace('name = "lull-app"', 'name = "lull-gone"'), /^app\/Cargo\.lock: no lull-app/],
    ['Cargo.toml', (text) => text.replace(`version = "${version}"`, `version = "${other}"`), /^app\/Cargo\.toml: /],
  ];
  for (const [label, change, expected] of cases) {
    const root = copyOfRepo(t);
    edit(root, label.split(' ')[0], change);
    const found = problems(root);
    assert.ok(found.length > 0, `${label}: nothing found`);
    assert.ok(found.some((line) => expected.test(line)), `${label}: ${found.join(' | ')}`);
  }
});

test('the check CLI exits non-zero and says which file', (t) => {
  const root = copyOfRepo(t);
  // The script reads the tree it lives in, so run a copy of it inside the temp tree.
  mkdirSync(join(root, 'scripts/ci'), { recursive: true });
  cpSync(join(repo, 'scripts/ci/version.mjs'), join(root, 'scripts/ci/version.mjs'));
  edit(root, 'app/Cargo.toml', (text) => text.replace(/^version = "[^"]*"/m, 'version = "9.9.9"'));
  const run = () => execFileSync('node', [join(root, 'scripts/ci/version.mjs'), 'check'], { encoding: 'utf8', stdio: 'pipe' });
  assert.throws(run, (error) => error.status === 1 && /app\/Cargo\.toml: version is 9\.9\.9/.test(error.stderr));
});

test('a bump with staging leaves the new changelog staged for `git commit -am`, and does without a repository', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  bump(root, target, { stage: true }); // not a repository: nothing to stage, no error
  assert.ok(existsSync(join(root, CHANGELOGS, `${versionCode(target)}.txt`)));

  const repoRoot = copyOfRepo(t);
  execFileSync('git', ['init', '-q'], { cwd: repoRoot });
  const next = nextPatch(readVersion(repoRoot));
  bump(repoRoot, next, { stage: true });
  const staged = execFileSync('git', ['diff', '--cached', '--name-only'], { cwd: repoRoot, encoding: 'utf8' }).trim();
  assert.equal(staged, `${CHANGELOGS}/${versionCode(next)}.txt`);
});
