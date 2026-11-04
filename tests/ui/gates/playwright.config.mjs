// The UI gates: performance budgets, key screens against their goldens, and accessibility, all on the fixture hub
// (../fixture-hub), so they need only Node, Chromium and WebKit: `mise run check:ui`. docs/context/quality.md#ui-gates.
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';

const here = dirname(fileURLToPath(import.meta.url));
const port = +(process.env.FIXTURE_PORT || 8790);
// Goldens are per platform: fonts and anti-aliasing differ between systems, so the committed ones are Linux's (CI)
// and a run elsewhere compares against pictures it made itself the first time (gitignored).
const capture = process.env.UI_CAPTURE === '1';
// GATES picks the projects a run takes: `perf` alone (its task lengths must not share the CPU with screenshots), `rest`
// (every other project, so a new gate runs in CI and in run.sh without being listed there), or, unset, all of them.
const group = process.env.GATES;
const inGroup = (p) => (group === 'perf' ? p.name === 'perf' : group === 'rest' ? p.name !== 'perf' : true);

export default defineConfig({
  testDir: here,
  outputDir: join(here, '..', 'test-results', 'gates'),
  snapshotPathTemplate: '{testDir}/goldens/{platform}/{arg}{ext}',
  // Linux without goldens is CI before the first capture: fail rather than pass on pictures nobody approved.
  updateSnapshots: capture ? 'all' : process.platform === 'linux' ? 'none' : 'missing',
  reporter: [['list'], ['html', { outputFolder: join(here, '..', 'report', 'gates'), open: 'never' }]],
  workers: process.env.CI ? 2 : 3,
  retries: 0,
  timeout: 90_000,
  expect: {
    toHaveScreenshot: { animations: 'disabled', caret: 'hide', maxDiffPixelRatio: 0.002 },
  },
  webServer: {
    command: `node ${JSON.stringify(join(here, '..', 'fixture-hub', 'server.mjs'))} ${port}`,
    url: `http://127.0.0.1:${port}/health`,
    reuseExistingServer: !process.env.CI,
  },
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    browserName: 'chromium',
    deviceScaleFactor: 1,
    locale: 'en-GB',
    timezoneId: 'UTC',
    launchOptions: { args: ['--autoplay-policy=no-user-gesture-required'] },
  },
  projects: [
    // run alone (run.sh): a gate that reads task lengths must not share the CPU with screenshots
    // no service worker under the perf traces: its install and caching would land in the startup numbers
    { name: 'perf', testMatch: 'perf.spec.mjs', use: { viewport: { width: 1440, height: 900 }, serviceWorkers: 'block' } },
    { name: 'screens', testMatch: 'screens.spec.mjs', fullyParallel: true },
    { name: 'a11y', testMatch: 'a11y.spec.mjs', fullyParallel: true, use: { viewport: { width: 1440, height: 900 } } },
    { name: 'covers', testMatch: 'covers.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
    { name: 'fit', testMatch: 'fit.spec.mjs', fullyParallel: true },
    // what loading Classic mode on first use must keep (a browser) and not change (the desktop app)
    { name: 'on-use', testMatch: 'on-use.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
    { name: 'now', testMatch: 'now.spec.mjs', fullyParallel: true },
    // long lists: every row works, drawn or not (a fixture hub of its own: it likes, downloads and reorders)
    { name: 'lists', testMatch: 'lists.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
    // loops at their own rate and held when asked, in Chromium and in WebKit (Linux's WebKitGTK app, Safari)
    { name: 'loops', testMatch: 'loops.spec.mjs', fullyParallel: true, use: { viewport: { width: 1440, height: 900 } } },
    // the desktop app's own pages in the listener's theme, handed over by main (no hub beside them)
    { name: 'app-pages', testMatch: 'app-pages.spec.mjs', use: { viewport: { width: 1000, height: 700 } } },
    // Theme Studio and import: a loop with no rate named, its rate added, the loop paced
    { name: 'theme-loops', testMatch: 'theme-loops.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
    // a theme's card shows a picture lull drew of it (ADR-0080), in Chromium and in WebKit (the Linux app's engine)
    { name: 'theme-pictures', testMatch: 'theme-pictures.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
    { name: 'theme-pictures-webkit', testMatch: 'theme-pictures.spec.mjs', use: { browserName: 'webkit', viewport: { width: 1440, height: 900 }, launchOptions: {} } },
    { name: 'loops-webkit', testMatch: 'loops.spec.mjs', fullyParallel: true, use: { browserName: 'webkit', viewport: { width: 1440, height: 900 }, launchOptions: {} } },
  ].filter(inGroup),
});

