// The UI gates: performance budgets, key screens against their goldens, and accessibility, all on the fixture hub
// (../fixture-hub), so they need only Node and Chromium: `mise run check:ui`. docs/context/quality.md#ui-gates.
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';

const here = dirname(fileURLToPath(import.meta.url));
const port = +(process.env.FIXTURE_PORT || 8790);
// Goldens are per platform: fonts and anti-aliasing differ between systems, so the committed ones are Linux's (CI)
// and a run elsewhere compares against pictures it made itself the first time (gitignored).
const capture = process.env.UI_CAPTURE === '1';

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
    { name: 'classic', testMatch: 'classic.spec.mjs', use: { viewport: { width: 1440, height: 900 } } },
  ],
});

