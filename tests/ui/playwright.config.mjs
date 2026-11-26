import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';
import { layoutRoutes, viewports } from './routes.mjs';

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  testDir: here,
  testMatch: 'matrix.spec.mjs',
  globalSetup: './global-setup.mjs',
  outputDir: join(here, 'test-results'),
  // results.json: which tests were flaky (failed, then passed on the retry), for scripts/playwright-flakes.mjs.
  reporter: [
    ['list'],
    ['html', { outputFolder: join(here, 'report'), open: 'never' }],
    ['json', { outputFile: join(here, 'report', 'results.json') }],
  ],
  workers: process.env.CI ? 3 : undefined,
  // Every test stands alone (its own page from the stored pairing), so CI can split them across machines (--shard).
  fullyParallel: true,
  // In CI a failing test runs once more on the same code; failing then passing makes it flaky, which doesn't hold the
  // landing and is reported to its owner (ADR-0095). Failing twice fails.
  retries: process.env.CI ? 1 : 0,
  timeout: 45_000,
  use: {
    baseURL: process.env.LULL_URL ?? 'http://127.0.0.1:8080',
    storageState: join(here, '.auth/state.json'),
    browserName: 'chromium',
    deviceScaleFactor: 1,
    locale: 'en-GB',
    timezoneId: 'UTC',
  },
  projects: [
    { name: 'phone', use: { viewport: viewports.phone, isMobile: true, hasTouch: true } },
    { name: 'desktop', use: { viewport: viewports.desktop } },
    ...['laptop', 'xwide', 'ultra'].map((name) => ({ name, grep: layoutRoutes, use: { viewport: viewports[name] } })),
  ],
});
