// After the nightly full run (.github/workflows/nightly.yml, docs/context/quality.md, The check: one gate): which of its jobs
// are red, and for each, whether path selection kept it off main. main.yml runs only the checks whose paths changed
// since its newest green run, so a check that is red here and that main last ran green, then skipped, broke behind a
// path skip: an escape, and its key in .github/ci/paths.yml misses a path the commits since touched.
//
//   node scripts/ci/nightly-escapes.mjs <run id> [--dry-run]     (mise run nightly:escapes <run id>)
//
// A red nightly on main opens or comments on one `nightly is red` issue (label nightly-red); a green one closes it.
// --dry-run, or a run that is not the nightly on main, prints the issue and writes nothing. Needs gh (GH_TOKEN with
// actions: read and issues: write), git with main's history, and GITHUB_REPOSITORY (else the checkout's remote).
// Exits 0 unless the script itself breaks: the red jobs already make the run red.
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';

export const LABEL = 'nightly-red';
export const TITLE = 'nightly is red';
// How many of main.yml's runs are read back for a red job's history: more than a day of landings.
const MAIN_RUNS = 30;
const FILES_MAX = 40;

// A job's name without its matrix part: `check / ui (1 of 4)` and a skipped `ui (${{ matrix.part }})` are both `ui`.
export function identity(name) {
  return name.replace(/ \(.*\)$/, '');
}

// Whether a main.yml job is the nightly job `id`. A called workflow that was skipped shows only its caller's job
// (`graphics`), never the jobs inside it (`graphics / linux`).
export function sameJob(id, mainName) {
  const other = identity(mainName);
  return other === id || id.startsWith(`${other} / `);
}

const isGate = (name) => name === 'gate' || name.endsWith(' / gate');

// The nightly's red jobs, once each by identity: failed, or stopped after they started (a timeout). The gate only
// repeats them, and the reporting job is still running.
export function redJobs(jobs) {
  const red = new Set();
  for (const job of jobs) {
    if (isGate(job.name)) continue;
    const stopped = job.conclusion === 'cancelled' && (job.steps ?? []).some((s) => s.conclusion === 'success');
    if (job.conclusion === 'failure' || stopped) red.add(identity(job.name));
  }
  return [...red];
}

// What main did with the red job `id` in `runs` (main.yml's, newest first, each `{ sha, jobs }`):
// - `nightly-only`: main's newest run has no such job (Lighthouse): main doesn't run it, whatever older runs did;
// - `main-red`: main's newest run of it was red too, and after.py has its `main is red` issue;
// - `ran`: main's newest run of it ran it green, so no path skip hid it (a flake, or an outside input: the newest
//   release, a download, the runner);
// - `escape`: main skipped it in `skipped` (newest first) since it last ran green at `since` (null when that is older
//   than the runs read, or than main having the job).
export function verdict(id, runs) {
  const skipped = [];
  for (const run of runs) {
    // A run a newer push replaced while it waited has no jobs: it says nothing either way.
    if (!run.jobs.length) continue;
    const mine = run.jobs.filter((job) => sameJob(id, job.name));
    if (!mine.length) break;
    if (mine.some((job) => job.conclusion === 'failure')) return { kind: 'main-red', since: run.sha, skipped };
    if (mine.every((job) => job.conclusion === 'skipped')) {
      skipped.push(run.sha);
      continue;
    }
    if (mine.some((job) => job.conclusion === 'success')) {
      return { kind: skipped.length ? 'escape' : 'ran', since: run.sha, skipped };
    }
    // cancelled while it waited: says nothing either way
  }
  return { kind: skipped.length ? 'escape' : 'nightly-only', since: null, skipped };
}

const short = (sha) => (sha ? sha.slice(0, 7) : '?');

// The issue's text for one red nightly. `found` is `[{ id, v, commits, files }]`; commits and files are git's lines.
export function report(run, found) {
  const escapes = found.filter((f) => f.v.kind === 'escape');
  const lines = [`The nightly full run is red: ${run.html_url} at \`${short(run.head_sha)}\`.`, ''];
  if (escapes.length) {
    lines.push(`**Escape:** path selection kept ${escapes.map((f) => `\`${f.id}\``).join(', ')} off main while it broke. ` +
      'Add the path that breaks it to its key in `.github/ci/paths.yml` (the commits main skipped it for are below).', '');
  }
  for (const { id, v, commits, files } of found) {
    if (v.kind === 'escape') {
      const from = v.since ? `since main last ran it green at \`${short(v.since)}\`` : `in all ${MAIN_RUNS} of main's runs read`;
      lines.push(`- \`${id}\`: **escape**. main skipped it on ${v.skipped.length} run(s) ${from}. The commits and files since:`, '');
      // Every line starts with a sha or four spaces, so none can close the fence.
      lines.push('```', ...commits, ...(files.length ? ['', ...files.map((f) => `    ${f}`)] : []), '```', '');
    } else if (v.kind === 'main-red') {
      lines.push(`- \`${id}\`: main is red on it too (its newest run of it, at \`${short(v.since)}\`): its \`main is red\` issue has it.`);
    } else if (v.kind === 'ran') {
      lines.push(`- \`${id}\`: not a path skip: main ran it green at \`${short(v.since)}\`. A flake, or an input that is not a ` +
        'path (the newest release, a download, the runner).');
    } else {
      lines.push(`- \`${id}\`: runs only in the nightly.`);
    }
  }
  return lines.join('\n').trimEnd() + '\n';
}

// --- GitHub and git ------------------------------------------------------------------------------------------------

const run = (cmd, args) => execFileSync(cmd, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });
let repoName;
const repo = () => (repoName ??= process.env.GITHUB_REPOSITORY ||
  run('gh', ['repo', 'view', '--json', 'nameWithOwner', '--jq', '.nameWithOwner']).trim());
const api = (path, ...args) => JSON.parse(run('gh', ['api', `repos/${repo()}/${path}`, ...args]) || 'null');
const jobsOf = (r) => api(`actions/runs/${r.id}/attempts/${r.run_attempt ?? 1}/jobs?per_page=100`).jobs ?? [];
const isAncestor = (a, b) => {
  try {
    run('git', ['merge-base', '--is-ancestor', a, b]);
    return true;
  } catch {
    return false;
  }
};

// main.yml's finished push runs that tested `sha` or a commit before it, newest first.
function mainRuns(sha) {
  const data = api(`actions/workflows/main.yml/runs?branch=main&event=push&status=completed&per_page=${MAIN_RUNS}`);
  return (data?.workflow_runs ?? [])
    .filter((r) => isAncestor(r.head_sha, sha))
    .map((r) => ({ sha: r.head_sha, jobs: jobsOf(r) }));
}

function commitsSince(since, sha) {
  const range = since ? [`${since}..${sha}`] : ['-n', '20', sha];
  const commits = run('git', ['log', '--format=%h  %s', ...range]).trim().split('\n').filter(Boolean);
  const files = since ? run('git', ['diff', '--name-only', since, sha]).trim().split('\n').filter(Boolean) : [];
  return {
    commits,
    files: files.length > FILES_MAX ? [...files.slice(0, FILES_MAX), `... and ${files.length - FILES_MAX} more`] : files,
  };
}

function openIssues() {
  return (api(`issues?state=open&labels=${LABEL}&per_page=100`) ?? []).filter((i) => !i.pull_request);
}

function main(argv) {
  const id = argv.find((a) => /^\d+$/.test(a));
  if (!id) throw new Error('usage: node scripts/ci/nightly-escapes.mjs <run id> [--dry-run]');
  const nightly = api(`actions/runs/${id}`);
  // Only the nightly on main speaks for main: a run by hand on a branch, or one read here as a stand-in, prints.
  const write = !argv.includes('--dry-run') && nightly.head_branch === 'main' &&
    (nightly.path ?? '').startsWith('.github/workflows/nightly.yml');
  const red = redJobs(jobsOf(nightly));
  if (!red.length) {
    console.log(`nightly-escapes: run ${id} has no red job`);
    for (const issue of write ? openIssues() : []) {
      run('gh', ['issue', 'close', String(issue.number), '--repo', repo(), '--comment',
        `The nightly is green again at ${short(nightly.head_sha)}: ${nightly.html_url}`]);
      console.log(`nightly-escapes: closed #${issue.number}`);
    }
    return;
  }
  const runs = mainRuns(nightly.head_sha);
  const found = red.map((job) => {
    const v = verdict(job, runs);
    return { id: job, v, ...(v.kind === 'escape' ? commitsSince(v.since, nightly.head_sha) : { commits: [], files: [] }) };
  });
  for (const f of found) console.log(`nightly-escapes: ${f.id}: ${f.v.kind}`);
  const body = report(nightly, found);
  if (!write) {
    console.log(`nightly-escapes: would file "${TITLE}":\n\n${body}`);
    return;
  }
  const open = openIssues().find((i) => i.title === TITLE);
  if (open) {
    run('gh', ['issue', 'comment', String(open.number), '--repo', repo(), '--body', body]);
    console.log(`nightly-escapes: commented on #${open.number}`);
  } else {
    run('gh', ['label', 'create', LABEL, '--repo', repo(), '--color', 'd73a4a', '--force',
      '--description', 'The nightly full run is red']);
    const url = run('gh', ['issue', 'create', '--repo', repo(), '--title', TITLE, '--label', LABEL, '--body', body]).trim();
    console.log(`nightly-escapes: opened ${url}`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  try {
    main(process.argv.slice(2));
  } catch (err) {
    console.error(`nightly-escapes: ${err.message}`);
    process.exit(1);
  }
}
