// Flaky tests (ADR-0095, docs/context/quality.md#flaky-checks): in CI a failing UI test runs once more on the same
// code, and one that failed and then passed is flaky. It doesn't hold the landing; this tells its owner. Each flaky
// test gets a warning on the run, a line in its summary, and an open issue labelled `flaky` (one per test, a comment
// for each time again), which its area's owner fixes and closes. Never fails the job: the verdict is the tests'.
//
//   node scripts/flaky.mjs <playwright results.json> <job name>
import { execFileSync } from 'node:child_process';
import { appendFileSync, existsSync, readFileSync } from 'node:fs';

const [report, job] = process.argv.slice(2);
if (!report || !existsSync(report)) {
  console.log(`no report at ${report}: nothing to say`);
  process.exit(0);
}

const flaky = [];
const walk = (suite, path) => {
  for (const spec of suite.specs ?? []) {
    for (const test of spec.tests ?? []) {
      if (test.status !== 'flaky') continue;
      const failure = test.results.find((r) => r.status !== 'passed')?.error?.message ?? '';
      flaky.push({
        title: [...path, spec.title].filter(Boolean).join(' › '),
        project: test.projectName,
        where: `${spec.file}:${spec.line}`,
        failure: failure.replace(/\u001b\[[0-9;]*m/g, '').split('\n').slice(0, 6).join('\n'),
      });
    }
  }
  for (const child of suite.suites ?? []) walk(child, [...path, child.title]);
};
for (const suite of JSON.parse(readFileSync(report, 'utf8')).suites ?? []) walk(suite, []);

if (!flaky.length) {
  console.log('no flaky tests');
  process.exit(0);
}

const run = process.env.GITHUB_SERVER_URL
  ? `${process.env.GITHUB_SERVER_URL}/${process.env.GITHUB_REPOSITORY}/actions/runs/${process.env.GITHUB_RUN_ID}`
  : '(no run)';
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const repo = process.env.GITHUB_REPOSITORY ?? '';
try {
  gh('label', 'create', 'flaky', '--repo', repo, '--color', 'd93f0b', '--description', 'A test that failed, then passed on its retry');
} catch {} // it exists
const summary = [`### Flaky in ${job}`, ''];
for (const t of flaky) {
  const name = `${t.title} [${t.project}]`;
  console.log(`::warning file=tests/ui/${t.where.split(':')[0]},title=Flaky test::${name} failed, then passed on its retry`);
  summary.push(`- \`${name}\` (${t.where}): failed, then passed on its retry`);
  const body = `${name}, at \`${t.where}\`, failed and then passed on its retry in ${job}: ${run}\n\n` +
    `It didn't hold the landing (ADR-0095). Fix the cause where it starts (docs/context/quality.md#flaky-checks), ` +
    `then close this.\n\n\`\`\`\n${t.failure}\n\`\`\``;
  const title = `Flaky test: ${name}`;
  try {
    const open = gh('issue', 'list', '--repo', repo, '--label', 'flaky', '--state', 'open',
      '--search', `"${title}" in:title`, '--json', 'number,title', '--jq', `.[] | select(.title == ${JSON.stringify(title)}) | .number`);
    if (open) gh('issue', 'comment', open.split('\n')[0], '--repo', repo, '--body', body);
    else gh('issue', 'create', '--repo', repo, '--title', title, '--label', 'flaky', '--body', body);
  } catch (e) {
    console.log(`::warning title=Flaky test::couldn't file ${name}: ${String(e.stderr || e.message).trim()}`);
  }
}
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary.join('\n') + '\n');
