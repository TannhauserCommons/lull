// scripts/ci/version.mjs: `mise run version` and `mise run bump`, and the check that every file holding lull's version agrees.
// A bump runs on a temp copy of the real holders, so these tests hold the real files to the script's reading of them.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { bump, problemsIn, problems, readVersion, trackedFiles, versionCode } from '../../scripts/ci/version.mjs';

const repo = resolve(import.meta.dirname, '..', '..');
const CHANGELOGS = 'fastlane/metadata/android/en-US/changelogs';
/** The files a bump changes today: the workspace members inherit the version, so their manifests stay as they are. */
const HOLDERS = ['Cargo.toml', 'app/Cargo.toml', 'app/tauri.android.conf.json', 'Cargo.lock', 'app/Cargo.lock'];

/** A temp tree holding copies of every real file that holds the version (the manifests, versionCode and lock files). */
function copyOfRepo(t) {
  const root = mkdtempSync(join(tmpdir(), 'lull-version-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const file of trackedFiles(repo)) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    cpSync(join(repo, file), join(root, file));
  }
  return root;
}

/** Every file under `root` (not git's own) with its bytes, for before and after. */
function snapshot(root, dir = root, into = new Map()) {
  for (const name of readdirSync(dir)) {
    const path = join(dir, name);
    if (name === '.git') continue;
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

  const { written } = bump(root, target);

  const stub = `${CHANGELOGS}/${versionCode(target)}.txt`;
  const after = snapshot(root);
  assert.deepEqual(changedFiles(before, after), [...HOLDERS, stub].sort());
  assert.deepEqual(written.sort(), [...HOLDERS, stub].sort());
  assert.equal(readFileSync(join(root, stub), 'utf8'), 'Smaller fixes.\n');
  assert.equal(readVersion(root), target);
  assert.deepEqual(problems(root), []);

  // Only version lines moved: swapping the new version back for the old one restores every holder byte for byte.
  const code = versionCode(target);
  for (const file of HOLDERS) {
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

test('bumping to the version the tree already carries changes nothing', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  bump(root, target);
  const before = snapshot(root);
  const { written } = bump(root, target);
  assert.deepEqual(written, []);
  assert.deepEqual(changedFiles(before, snapshot(root)), []);
});

test('an existing changelog for the code is kept', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  const stub = join(root, CHANGELOGS, `${versionCode(target)}.txt`);
  mkdirSync(dirname(stub), { recursive: true });
  writeFileSync(stub, '* Written by hand\n');
  const { written } = bump(root, target);
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
  for (const file of HOLDERS) {
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
    ['app/Cargo.lock (no lull-app)', (text) => text.replace('name = "lull-app"', 'name = "lull-gone"'), /^app\/Cargo\.lock: no entry for lull-app/],
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

/** The lock text without the [[package]] block named `name`. */
const withoutPackage = (text, name) => text.replace(new RegExp(`\\[\\[package\\]\\]\\nname = "${name}"\\n(?:(?!\\[\\[package\\]\\])[^\\n]*\\n)*`), '');

test('the check holds every workspace member to the version, in its manifest and in the locks', (t) => {
  const cases = [
    ['a member with a version of its own', 'crates/core/Cargo.toml', (text) => text.replace('version.workspace = true', 'version = "0.1.0"'), /^crates\/core\/Cargo\.toml: version is 0\.1\.0/],
    ['a member with no [package] name', 'crates/core/Cargo.toml', (text) => text.replace(/^name = .*\n/m, ''), /^crates\/core\/Cargo\.toml: no \[package\] name/],
    ['a member of the workspace with no manifest', 'Cargo.toml', (text) => text.replace('"crates/core",', '"crates/core",\n  "crates/gone",'), /^crates\/gone\/Cargo\.toml: missing/],
    ['a member missing from Cargo.lock', 'Cargo.lock', (text) => withoutPackage(text, 'lull-youtube'), /^Cargo\.lock: no entry for lull-youtube/],
    ['a lock with one lull entry', 'Cargo.lock', (text) => ['lull-collection', 'lull-discovery', 'lull-lastfm', 'lull-library', 'lull-lyrics', 'lull-server', 'lull-skins', 'lull-soulseek', 'lull-subsonic', 'lull-themes', 'lull-youtube'].reduce(withoutPackage, text), /^Cargo\.lock: no entry for lull-collection/],
    ['a member the app reaches missing from app/Cargo.lock', 'app/Cargo.lock', (text) => withoutPackage(text, 'lull-core'), /^app\/Cargo\.lock: no entry for lull-core/],
    ['a member the app reaches through another one', 'app/Cargo.lock', (text) => withoutPackage(text, 'lull-soulseek'), /^app\/Cargo\.lock: no entry for lull-soulseek/],
  ];
  for (const [label, file, change, expected] of cases) {
    const root = copyOfRepo(t);
    edit(root, file, change);
    const found = problems(root);
    assert.ok(found.some((line) => expected.test(line)), `${label}: ${found.join(' | ')}`);
  }
});

test('a member that carries its own version is bumped with the rest', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  edit(root, 'crates/core/Cargo.toml', (text) => text.replace('version.workspace = true', `version = "${version}"`));
  assert.deepEqual(problems(root), []);
  const target = nextPatch(version);
  const { written } = bump(root, target);
  assert.ok(written.includes('crates/core/Cargo.toml'));
  assert.match(readFileSync(join(root, 'crates/core/Cargo.toml'), 'utf8'), new RegExp(`^version = "${target.replaceAll('.', '\\.')}"$`, 'm'));
  assert.deepEqual(problems(root), []);
});

test('a Cargo.toml version that is not X.Y.Z is named as the problem, not as "not newer"', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  edit(root, 'Cargo.toml', (text) => text.replace(`version = "${version}"`, 'version = "0.3.0-rc1"'));
  assert.throws(() => bump(root, nextPatch(version)), (error) => /Cargo\.toml/.test(error.message) && /0\.3\.0-rc1/.test(error.message) && /not X\.Y\.Z/.test(error.message));
});

test('a bump that cannot finish leaves every holder as it was: no folder for the changelog', (t) => {
  const root = copyOfRepo(t);
  writeFileSync(join(root, 'fastlane'), 'a file where the changelog folder goes\n');
  const target = nextPatch(readVersion(root));
  const before = snapshot(root);
  assert.throws(() => bump(root, target), /ENOTDIR|EEXIST|not a directory/i);
  assert.deepEqual(changedFiles(before, snapshot(root)), [], 'a holder or a temp file was left behind');

  rmSync(join(root, 'fastlane'));
  bump(root, target);
  assert.deepEqual(problems(root), []);
});

test('a bump that cannot write a temp file leaves every holder as it was', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  // The last holder's temp name is taken, so the ones before it were already written when this one fails.
  mkdirSync(join(root, `app/Cargo.lock.${process.pid}.tmp`));
  const before = snapshot(root);
  assert.throws(() => bump(root, target), /EISDIR|EEXIST/);
  rmSync(join(root, `app/Cargo.lock.${process.pid}.tmp`), { recursive: true });
  assert.deepEqual(changedFiles(before, snapshot(root)), [], 'a holder or a temp file was left behind');
  bump(root, target);
  assert.deepEqual(problems(root), []);
});

test('a read-only holder is refused before anything is written', (t) => {
  const root = copyOfRepo(t);
  const target = nextPatch(readVersion(root));
  const file = join(root, 'app/Cargo.lock');
  chmodSync(file, 0o444);
  const before = snapshot(root);
  assert.throws(() => bump(root, target), /app\/Cargo\.lock is read-only/);
  assert.deepEqual(changedFiles(before, snapshot(root)), []);
  chmodSync(file, 0o644);
  bump(root, target);
  assert.deepEqual(problems(root), []);
});

test('a bump whose staging fails writes nothing, and the rerun finishes it', (t) => {
  const root = copyOfRepo(t);
  execFileSync('git', ['init', '-q'], { cwd: root });
  const target = nextPatch(readVersion(root));
  const before = snapshot(root);
  writeFileSync(join(root, '.git/index.lock'), ''); // git add cannot take the index
  assert.throws(() => bump(root, target, { stage: true }), /git/);
  assert.deepEqual(changedFiles(before, snapshot(root)), [], 'holders or the stub changed although git add failed');

  rmSync(join(root, '.git/index.lock'));
  bump(root, target, { stage: true });
  assert.deepEqual(problems(root), []);
  assert.equal(execFileSync('git', ['diff', '--cached', '--name-only'], { cwd: root, encoding: 'utf8' }).trim(), `${CHANGELOGS}/${versionCode(target)}.txt`);
});

test('a rerun at a version the tree already partly carries completes it', (t) => {
  const root = copyOfRepo(t);
  const version = readVersion(root);
  const target = nextPatch(version);
  // What a bump that died halfway used to leave: Cargo.toml and one lock moved, the rest not, and no stub.
  edit(root, 'Cargo.toml', (text) => text.replace(`version = "${version}"`, `version = "${target}"`));
  edit(root, 'Cargo.lock', (text) => text.replace(/(name = "lull-[a-z]+"\nversion = ")[^"]*/g, `$1${target}`));
  assert.ok(problems(root).length > 0);

  execFileSync('git', ['init', '-q'], { cwd: root });
  const { written, staged } = bump(root, target, { stage: true });
  assert.deepEqual(problems(root), []);
  assert.ok(staged);
  assert.ok(written.includes(`${CHANGELOGS}/${versionCode(target)}.txt`) && written.includes('app/Cargo.toml') && !written.includes('Cargo.toml'));
  assert.equal(execFileSync('git', ['diff', '--cached', '--name-only'], { cwd: root, encoding: 'utf8' }).trim(), `${CHANGELOGS}/${versionCode(target)}.txt`);

  // Rerunning once more has nothing to do, and stages nothing new.
  assert.deepEqual(bump(root, target, { stage: true }).written, []);
});

test('the stub is staged in the repository lull\'s folder sits in, an enclosing one included', (t) => {
  const outer = mkdtempSync(join(tmpdir(), 'lull-version-outer-'));
  t.after(() => rmSync(outer, { recursive: true, force: true }));
  execFileSync('git', ['init', '-q'], { cwd: outer });
  const root = join(outer, 'unpacked');
  mkdirSync(root);
  for (const file of trackedFiles(repo)) {
    mkdirSync(dirname(join(root, file)), { recursive: true });
    cpSync(join(repo, file), join(root, file));
  }
  const target = nextPatch(readVersion(root));
  const { staged } = bump(root, target, { stage: true });
  assert.equal(staged, true);
  // `git commit -am` from lull's folder commits to this repository, so that is where the new file must be staged.
  assert.equal(execFileSync('git', ['diff', '--cached', '--name-only'], { cwd: outer, encoding: 'utf8' }).trim(), `unpacked/${CHANGELOGS}/${versionCode(target)}.txt`);
});

test('a workspace list the script cannot resolve is refused by the check and by bump, naming the file and the entry', (t) => {
  const cases = [
    ['a glob in members', 'Cargo.toml', (text) => text.replace('members = [', 'members = [\n  "crates/*",'), /^Cargo\.toml: workspace\.members entry "crates\/\*" is a glob/],
    ['a glob in exclude', 'Cargo.toml', (text) => text.replace('exclude = ["app"]', 'exclude = ["app", "tools/*"]'), /^Cargo\.toml: workspace\.exclude entry "tools\/\*" is a glob/],
    ['a member folder with no manifest', 'Cargo.toml', (text) => text.replace('"crates/core",', '"crates/core",\n  "crates/gone",'), /^crates\/gone\/Cargo\.toml: missing, but Cargo\.toml lists its folder/],
    ['an excluded folder with no manifest', 'Cargo.toml', (text) => text.replace('exclude = ["app"]', 'exclude = ["app", "tools"]'), /^tools\/Cargo\.toml: missing/],
    ['members as a dotted key', 'Cargo.toml', (text) => `workspace.members = ["crates/core"]\n${text}`, /^Cargo\.toml: workspace is written as a dotted or inline key/],
    ['members that is not a list of strings', 'Cargo.toml', (text) => text.replace(/members = \[[^\]]*\]/, 'members = { core = "crates/core" }'), /^Cargo\.toml: workspace\.members is not a plain list of strings/],
    ['a renamed member in the root manifest', 'Cargo.toml', (text) => `${text}\n[workspace.dependencies.core]\npath = "crates/core"\npackage = "lull-core"\n`, /^Cargo\.toml: renames a dependency on lull-core/],
  ];
  for (const [label, file, change, expected] of cases) {
    const root = copyOfRepo(t);
    edit(root, file, change);
    const found = problems(root);
    assert.ok(found.some((line) => expected.test(line)), `check, ${label}: ${found.join(' | ')}`);
    const before = snapshot(root);
    assert.throws(() => bump(root, nextPatch(readVersion(root))), new RegExp(expected.source.replace(/^\^/, ''), 'm'), `bump, ${label}`);
    assert.deepEqual(changedFiles(before, snapshot(root)), [], `bump, ${label}, wrote files`);
  }
});

test('a glob is reported although the member behind it is stale and its lock entry gone', (t) => {
  const root = copyOfRepo(t);
  edit(root, 'Cargo.toml', (text) => text.replace(/members = \[[^\]]*\]/, 'members = ["crates/*"]'));
  edit(root, 'crates/core/Cargo.toml', (text) => text.replace('version.workspace = true', 'version = "9.9.9"'));
  edit(root, 'Cargo.lock', (text) => withoutPackage(text, 'lull-core'));
  const found = problems(root);
  assert.ok(found.some((line) => /^Cargo\.toml: .*is a glob/.test(line)), found.join(' | '));
});

test('the app workspace\'s reach follows a dependency written as a table, and one renamed with package =', () => {
  const manifest = (name, deps = '') => `[package]\nname = "${name}"\nversion = "1.0.0"\n${deps}`;
  const lock = (names) => names.map((name) => `[[package]]\nname = "${name}"\nversion = "1.0.0"\n`).join('\n');
  const tree = (appDeps, appLock) => ({
    'Cargo.toml': '[workspace]\nmembers = ["crates/a"]\nexclude = ["app"]\n\n[workspace.package]\nversion = "1.0.0"\n',
    'crates/a/Cargo.toml': '[package]\nname = "lull-a"\nversion.workspace = true\n',
    'app/Cargo.toml': manifest('lull-app', appDeps),
    'app/tauri.android.conf.json': '{"bundle":{"android":{"versionCode":1000000}}}',
    'Cargo.lock': lock(['lull-a']),
    'app/Cargo.lock': lock(appLock),
  });
  assert.deepEqual(problemsIn(tree('\n[dependencies]\nlull-a = { path = "../crates/a" }\n', ['lull-app', 'lull-a'])), []);
  for (const [label, deps] of [
    ['a [dependencies.x] table', '\n[dependencies.lull-a]\npath = "../crates/a"\n'],
    ['a target table', '\n[target.\'cfg(unix)\'.dependencies.lull-a]\npath = "../crates/a"\n'],
    ['a rename', '\n[dependencies]\nsomething = { path = "../crates/a", package = "lull-a" }\n'],
  ]) {
    assert.deepEqual(problemsIn(tree(deps, ['lull-app'])), ['app/Cargo.lock: no entry for lull-a'], `${label} was not followed`);
    assert.deepEqual(problemsIn(tree(deps, ['lull-app', 'lull-a'])), [], label);
  }
});
