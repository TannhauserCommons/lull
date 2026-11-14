// Flaky Playwright tests, for the `flakes/` folder of the shared CI (docs/context/quality.md, Check in shadow). In CI a
// failing UI test runs once more on the same code (`retries` in both Playwright configs); one that failed and then passed
// is flaky. It doesn't fail the job, and its owner hears of it: this writes it to flakes/playwright.txt, the job uploads
// the folder, and the `after` workflow files an issue per test (.github/ci/after.py). No token is needed here.
//
//   node scripts/playwright-flakes.mjs <results.json>...
//
// One test id a line: `<file>::<describe › title> [<project>]`, the file from the repo root. No file when nothing was
// flaky. A report that isn't there (the run died before it wrote one) is said and skipped: that job is red already.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

// The ids of the tests a Playwright JSON report marks flaky, once each. Files are named from `root`.
export function flakyTests(report, root = process.cwd()) {
  const base = report.config?.rootDir;
  const found = new Set();
  const walk = (suite, path) => {
    for (const spec of suite.specs ?? []) {
      for (const test of spec.tests ?? []) {
        if (test.status !== 'flaky') continue;
        const file = base ? relative(root, resolve(base, spec.file)).replaceAll('\\', '/') : spec.file;
        const title = [...path, spec.title].filter(Boolean).join(' › ');
        found.add(`${file}::${title} [${test.projectName}]`.replace(/\s+/g, ' '));
      }
    }
    // The top-level suite is the file; only the describes under it are part of a test's name.
    for (const child of suite.suites ?? []) walk(child, [...path, child.title]);
  };
  for (const suite of report.suites ?? []) walk(suite, []);
  return [...found];
}

// Reads the reports, writes `<dir>/playwright.txt` when any test was flaky, and returns the ids.
export function writeFlakes(reports, dir = 'flakes', root = process.cwd()) {
  const ids = new Set();
  for (const path of reports) {
    if (!existsSync(path)) {
      console.log(`no report at ${path}: nothing to say`);
      continue;
    }
    for (const id of flakyTests(JSON.parse(readFileSync(path, 'utf8')), root)) ids.add(id);
  }
  if (!ids.size) {
    console.log('no flaky tests');
    return [];
  }
  mkdirSync(dir, { recursive: true });
  writeFileSync(resolve(dir, 'playwright.txt'), [...ids].join('\n') + '\n');
  for (const id of ids) console.log(`::warning title=Flaky test::${id} failed, then passed on its retry`);
  return [...ids];
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const reports = process.argv.slice(2);
  if (!reports.length) {
    console.error('usage: node scripts/playwright-flakes.mjs <results.json>...');
    process.exit(2);
  }
  writeFlakes(reports);
}
