// The nightly's escape report (scripts/ci/nightly-escapes.mjs): a check red in the nightly that main last ran green and
// then skipped is an escape of path selection; one main ran green at its newest run, or is red on too, or never runs,
// is not. Job names are matched as GitHub writes them: a matrix part, a skipped matrix job, a skipped called workflow.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { identity, sameJob, redJobs, verdict, report } from '../../scripts/ci/nightly-escapes.mjs';

const job = (name, conclusion) => ({ name, conclusion });
const mainRun = (sha, ...jobs) => ({ sha, jobs });

test('a job is named without its matrix part, as run or as skipped', () => {
  assert.equal(identity('check / ui (1 of 4)'), 'check / ui');
  assert.equal(identity('check / ui (${{ matrix.part }})'), 'check / ui');
  assert.equal(identity('check / slskd-build'), 'check / slskd-build');
  assert.ok(sameJob('graphics / linux', 'graphics'), 'a skipped called workflow shows only its caller job');
  assert.ok(!sameJob('check / app', 'check / appx'));
  assert.ok(!sameJob('check / image', 'image'), "main's own publish job is not check's image");
});

test('the red jobs leave out the gate, and count a job stopped after it started', () => {
  const red = redJobs([
    job('check / ui (1 of 4)', 'failure'), job('check / ui (2 of 4)', 'failure'), job('check / gate', 'failure'),
    job('check / aur', 'success'),
    { name: 'graphics / linux', conclusion: 'cancelled', steps: [{ conclusion: 'success' }] },
    { name: 'lighthouse (1 of 2)', conclusion: 'cancelled', steps: [] },
  ]);
  assert.deepEqual(red, ['check / ui', 'graphics / linux']);
});

test('red behind a path skip after a green run is an escape, with the runs that skipped it', () => {
  const runs = [
    mainRun('c3', job('check / ui (${{ matrix.part }})', 'skipped')),
    mainRun('c2', job('check / ui (${{ matrix.part }})', 'skipped')),
    mainRun('c1', job('check / ui (1 of 4)', 'success'), job('check / ui (2 of 4)', 'success')),
  ];
  assert.deepEqual(verdict('check / ui', runs), { kind: 'escape', since: 'c1', skipped: ['c3', 'c2'] });
  const graphics = [mainRun('c2', job('graphics', 'skipped')), mainRun('c1', job('graphics / linux', 'success'))];
  assert.equal(verdict('graphics / linux', graphics).kind, 'escape');
});

test('red that main ran green, is red on, or never runs is not an escape', () => {
  assert.equal(verdict('check / aur', [mainRun('c2', job('check / aur', 'success'))]).kind, 'ran');
  assert.equal(verdict('check / aur', [mainRun('c2', job('check / aur', 'skipped')), mainRun('c1', job('check / aur', 'failure'))]).kind, 'main-red');
  assert.equal(verdict('lighthouse', [mainRun('c1', job('check / aur', 'success'))]).kind, 'nightly-only');
  // A job main stopped running: its red there before is not main's red now.
  assert.equal(verdict('lighthouse', [mainRun('c2', job('check / aur', 'success')), mainRun('c1', job('lighthouse (2 of 2)', 'failure'))]).kind, 'nightly-only');
  // A run replaced while it waited has no jobs: it is passed over, not counted as a skip.
  assert.equal(verdict('check / aur', [mainRun('c2'), mainRun('c1', job('check / aur', 'success'))]).kind, 'ran');
});

test('the issue leads with the escape and names the paths.yml key to fix', () => {
  const body = report({ html_url: 'https://example.invalid/run/1', head_sha: 'abcdef0123' }, [
    { id: 'check / ui', v: { kind: 'escape', since: 'c1aaaaaa', skipped: ['c3', 'c2'] }, commits: ['c3c3c3c  feat(web): x'], files: ['web/x.js'] },
    { id: 'lighthouse', v: { kind: 'nightly-only', since: null, skipped: [] }, commits: [], files: [] },
  ]);
  assert.match(body, /\*\*Escape:\*\* path selection kept `check \/ ui` off main/);
  assert.match(body, /\.github\/ci\/paths\.yml/);
  assert.match(body, /skipped it on 2 run\(s\) since main last ran it green at `c1aaaaa`/);
  assert.match(body, /```\nc3c3c3c  feat\(web\): x\n\n    web\/x\.js\n```/);
  assert.match(body, /`lighthouse`: runs only in the nightly/);
});
