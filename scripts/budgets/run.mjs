// `mise run budgets [base]`: the shipped bytes of each route's start on this tree and on its merge-base, and whether the
// growth is within the route's allowance and the size under its ceiling (docs/context/quality.md, Check in shadow).
//
// (mise hands the optional base to its task as `usage_base`, in the environment, so cmd on Windows reads it too.)
// The base is the merge-base with origin/main (a pull request's merge commit and a queue branch both have one), or the
// revision given. This tree is measured as it is on disk. The harness that judges comes from the base: its measure.mjs
// and decide.mjs measure both trees, and its budgets file holds the limits, so a change cannot loosen what it is
// judged by. The first run, when the base has none of it, uses this change's own and says so.
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const FILE = 'scripts/budgets/shipped-bytes.json';
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();

function baseRev(given) {
  if (given) return git('rev-parse', '--verify', `${given}^{commit}`);
  for (const main of ['origin/main', 'main']) {
    try { return git('merge-base', 'HEAD', main); } catch { /* try the next */ }
  }
  throw new Error('no merge-base with origin/main or main: fetch the history (CI: fetch-depth: 0), or name a base: mise run budgets <rev>');
}

// The base's files that measuring needs, in a folder of their own: git archive writes what is committed, the same bytes
// on every OS (a working tree is converted to the machine's line endings; the archive is not).
function extractBase(rev, into) {
  const tracked = git('ls-tree', '-r', '--name-only', rev, '--', 'scripts/budgets').split('\n').filter(Boolean);
  const paths = ['web', 'scripts/web-dist', ...(tracked.length ? ['scripts/budgets'] : [])];
  const archive = spawnSync('git', ['archive', '--format=tar', rev, '--', ...paths], { cwd: root, maxBuffer: 1 << 28 });
  if (archive.status) throw new Error(`git archive ${rev}: ${archive.stderr}`);
  // cwd, not `-C`: GNU tar reads `C:` in a Windows path as a remote host.
  const untar = spawnSync('tar', ['-x'], { cwd: into, input: archive.stdout });
  if (untar.status) throw new Error(`tar: ${untar.stderr}`);
}

const baseTree = mkdtempSync(join(tmpdir(), 'lull-budgets-base-'));
try {
  const rev = baseRev(process.argv[2] || process.env.usage_base);
  extractBase(rev, baseTree);
  const hasHarness = ['measure.mjs', 'decide.mjs', 'shipped-bytes.json'].every((f) => existsSync(join(baseTree, 'scripts/budgets', f)));
  const harness = join(hasHarness ? baseTree : root, 'scripts/budgets');
  const { measureTree, readModel } = await import(pathToFileURL(join(harness, 'measure.mjs')).href);
  const { decide, report } = await import(pathToFileURL(join(harness, 'decide.mjs')).href);
  const changeModel = readModel(root);
  if (hasHarness) console.log(`budgets: the change against ${rev.slice(0, 12)}, judged by that commit's harness and limits`);
  else console.log(`budgets: ${rev.slice(0, 12)} has no budgets harness yet (the first run): both trees are measured, and judged, by this change's own`);
  const base = measureTree(baseTree, hasHarness ? readModel(baseTree) : changeModel);
  const change = measureTree(root, changeModel);
  const limits = hasHarness ? readModel(baseTree) : changeModel;
  const { ok, lines } = report(decide({ base, change, limits }), FILE);
  console.log(lines.join('\n'));
  process.exitCode = ok ? 0 : 1;
} catch (err) {
  console.error(`budgets: ${err.message}`);
  process.exitCode = 1;
} finally {
  rmSync(baseTree, { recursive: true, force: true });
}
