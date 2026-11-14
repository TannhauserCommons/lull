// The flaky Playwright tests CI hands to the shared `after` workflow (scripts/playwright-flakes.mjs): a test that failed and
// then passed on its retry is named once, in the form after.py reads, and nothing else is: not a test that failed twice, one
// that passed first time, or one that was skipped. Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { flakyTests, writeFlakes } from '../../scripts/playwright-flakes.mjs';

const root = resolve('/repo');
const spec = (title, status, projectName = 'desktop') => ({ title, file: 'fit.spec.mjs', tests: [{ projectName, status }] });
const report = {
  config: { rootDir: join(root, 'tests', 'ui', 'gates') },
  suites: [{
    title: 'fit.spec.mjs',
    specs: [spec('opens', 'flaky')],
    suites: [{
      title: 'chrome',
      specs: [spec('fits at 360', 'expected'), spec('fits at 390', 'flaky', 'phone'), spec('never fits', 'unexpected'), spec('later', 'skipped')],
      suites: [{ title: 'in Petal', specs: [spec('keeps  its   rows', 'flaky')] }],
    }],
  }],
};

test('only a test that failed and then passed is flaky, named by file, describes, title and project', () => {
  assert.deepEqual(flakyTests(report, root), [
    'tests/ui/gates/fit.spec.mjs::opens [desktop]',
    'tests/ui/gates/fit.spec.mjs::chrome › fits at 390 [phone]',
    'tests/ui/gates/fit.spec.mjs::chrome › in Petal › keeps its rows [desktop]',
  ]);
});

test('a test flaky in two projects is two ids, and one flaky twice in a report is one', () => {
  const two = { ...report, suites: [{ title: 'a', specs: [spec('x', 'flaky', 'a'), spec('x', 'flaky', 'b'), spec('x', 'flaky', 'a')] }] };
  assert.deepEqual(flakyTests(two, root), ['tests/ui/gates/fit.spec.mjs::x [a]', 'tests/ui/gates/fit.spec.mjs::x [b]']);
});

test('the ids go to flakes/playwright.txt, one a line, and a report that is not there is skipped', () => {
  const dir = mkdtempSync(join(tmpdir(), 'flakes-'));
  const results = join(dir, 'results.json');
  writeFileSync(results, JSON.stringify(report));
  const written = writeFlakes([join(dir, 'missing.json'), results], join(dir, 'flakes'), root);
  assert.equal(written.length, 3);
  assert.equal(readFileSync(join(dir, 'flakes', 'playwright.txt'), 'utf8'), written.join('\n') + '\n');
});

test('nothing flaky writes no file', () => {
  const dir = mkdtempSync(join(tmpdir(), 'flakes-'));
  const results = join(dir, 'results.json');
  writeFileSync(results, JSON.stringify({ config: {}, suites: [{ title: 'a', specs: [spec('x', 'expected')] }] }));
  assert.deepEqual(writeFlakes([results], join(dir, 'flakes'), root), []);
  assert.equal(existsSync(join(dir, 'flakes')), false);
});
