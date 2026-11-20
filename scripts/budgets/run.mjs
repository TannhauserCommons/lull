// `mise run budgets [base]`: the shipped bytes of each route's start on this tree and on its merge-base, and whether the
// growth is within the allowance (docs/context/quality.md, Budgets check).
//
// The base is the merge-base with origin/main (a pull request's merge commit and a queue branch both have one), or the
// revision given (mise hands it to its task as `usage_base`, in the environment, so cmd on Windows reads it too). This
// tree is measured as it is on disk. Which side's files do what:
//   - the merge-base's harness (list.mjs, measure.mjs, decide.mjs) lists and sizes both trees and judges, and its budgets
//     file holds the limits of every route it knows, so a change cannot loosen what judges it;
//   - each tree is built and listed by a real browser against its own build and its own fixture hub, so a change that
//     makes a route load a new file is charged for it in the same change, and one that stops loading a file is credited;
//   - the routes measured are the ones each side's budgets file names, and the change is also asked about the routes the
//     merge-base budgets that its file drops: a screen that is gone lists as removed, one that still exists fails; a
//     route the merge-base doesn't know is judged by the change's own entry and printed as new;
//   - when nothing grew on a route both sides measured, the change's own budgets file judges (the owner's changes to the
//     limits themselves, over a main already red, or removing a route), and the table says so;
//   - a merge-base with no harness at all is judged by the change's own, but only for the change that adds the harness.
// The entry script itself runs from the change's tree, so the check's own files are protected paths: a change to them is
// the owner's to look at (quality.md).
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const FILE = 'scripts/budgets/shipped-bytes.json';
const HARNESS = ['measure.mjs', 'list.mjs', 'decide.mjs', 'shipped-bytes.json'];
const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();

// A finished child process, or an error saying how it did not finish: a program that can't start, one that was killed
// and one that overflowed its output buffer all leave `status` null, which `!== 0` alone would read as no news.
export function checked(result, what) {
  if (result.error) throw new Error(`${what}: ${result.error.message}`);
  if (result.signal) throw new Error(`${what} was ended by ${result.signal}`);
  if (result.status !== 0) throw new Error(`${what} failed (${result.status}): ${String(result.stderr ?? '').trim()}`);
  return result;
}

// Whose harness measures and judges: the merge-base's when it has one, else the change's own, which is allowed only to the
// change that adds the harness (otherwise a base that lacks it, a stale one or a wrong revision, would let a change judge itself).
export function chooseHarness({ baseHasHarness, addedByChange, rev }) {
  if (baseHasHarness) return 'base';
  if (addedByChange) return 'own';
  throw new Error(`${rev.slice(0, 12)} has no budgets harness (scripts/budgets) and this change does not add it, so nothing but this change would judge it: rebase on main, or name a base that has it (mise run budgets <rev>)`);
}

function baseRev(given) {
  if (given) return git('rev-parse', '--verify', `${given}^{commit}`);
  for (const main of ['origin/main', 'main']) {
    try { return git('merge-base', 'HEAD', main); } catch { /* try the next */ }
  }
  throw new Error('no merge-base with origin/main or main: fetch the history (CI: fetch-depth: 0), or name a base: mise run budgets <rev>');
}

// Whether this change adds the harness: a commit between the base and HEAD does, or it is not committed yet.
function addsHarness(rev) {
  const measure = 'scripts/budgets/measure.mjs';
  return git('log', '--diff-filter=A', '--format=%H', `${rev}..HEAD`, '--', measure) !== '' || git('status', '--porcelain', '--', measure) !== '';
}

// The base's files that measuring needs, in a folder of their own: git archive writes what is committed, the same bytes
// on every OS (a working tree is converted to the machine's line endings; the archive is not).
function extractBase(rev, into) {
  const tracked = git('ls-tree', '-r', '--name-only', rev, '--', 'scripts/budgets').split('\n').filter(Boolean);
  const paths = ['web', 'scripts/web-dist', 'tests/ui/fixture-hub', ...(tracked.length ? ['scripts/budgets'] : [])];
  const archive = checked(spawnSync('git', ['archive', '--format=tar', rev, '--', ...paths], { cwd: root, maxBuffer: 1 << 28 }), `git archive ${rev.slice(0, 12)}`);
  // cwd, not `-C`: GNU tar reads `C:` in a Windows path as a remote host.
  checked(spawnSync('tar', ['-x'], { cwd: into, input: archive.stdout }), 'tar');
}

async function main() {
  const baseTree = mkdtempSync(join(tmpdir(), 'lull-budgets-base-'));
  try {
    const rev = baseRev(process.argv[2] || process.env.usage_base);
    extractBase(rev, baseTree);
    const baseHasHarness = HARNESS.every((f) => existsSync(join(baseTree, 'scripts/budgets', f)));
    const whose = chooseHarness({ baseHasHarness, addedByChange: !baseHasHarness && addsHarness(rev), rev });
    const harness = join(whose === 'base' ? baseTree : root, 'scripts/budgets');
    const { measureTree, readLimits, loadChromium } = await import(pathToFileURL(join(harness, 'measure.mjs')).href);
    const { decide, report } = await import(pathToFileURL(join(harness, 'decide.mjs')).href);
    const own = readLimits(root);
    const model = whose === 'base' ? readLimits(baseTree) : own;
    if (whose === 'base') console.log(`budgets: the change against ${rev.slice(0, 12)}, judged by that commit's harness and limits`);
    else console.log(`budgets: ${rev.slice(0, 12)} has no budgets harness: this change adds it, so both trees are measured, and judged, by this change's own`);
    const chromium = await loadChromium(root);
    const budgeted = Object.keys(model.routes), listed = Object.keys(own.routes);
    const base = await measureTree(baseTree, budgeted, chromium);
    // The routes the merge-base budgets are listed in the change too, to see whether their screens are still there.
    const dropped = budgeted.filter((route) => !listed.includes(route));
    const change = await measureTree(root, [...listed, ...dropped], chromium, { mayBeGone: dropped });
    const { ok, lines } = report(decide({ base, change, limits: model, ownLimits: own }), FILE);
    console.log(lines.join('\n'));
    process.exitCode = ok ? 0 : 1;
  } catch (err) {
    console.error(`budgets: ${err.message}`);
    process.exitCode = 1;
  } finally {
    rmSync(baseTree, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) await main();
