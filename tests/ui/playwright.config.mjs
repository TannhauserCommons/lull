import { existsSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { defineConfig } from '@playwright/test';
import { viewports } from './routes.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const goldens = join(here, 'goldens');
// With no goldens committed (or UI_CAPTURE=1) a run writes them instead of comparing: the pictures
// are only comparable when they come from the CI image, so the integrator takes them from the
// run's `goldens` artifact (docs/context/quality.md).
// Every worker reads this file again, and by then the first goldens exist: decide once and hand the
// answer down through the environment.
const capture = process.env.UI_CAPTURE === '1' || !existsSync(goldens) || readdirSync(goldens).length === 0;
if (capture) process.env.UI_CAPTURE = '1';

export default defineConfig({
  testDir: here,
  testMatch: 'matrix.spec.mjs',
  globalSetup: './global-setup.mjs',
  snapshotPathTemplate: '{testDir}/goldens/{projectName}/{arg}{ext}',
  updateSnapshots: capture ? 'all' : 'none',
  outputDir: join(here, 'test-results'),
  reporter: [['list'], ['html', { outputFolder: join(here, 'report'), open: 'never' }]],
  workers: process.env.CI ? 3 : undefined,
  retries: 0,
  timeout: 45_000,
  expect: {
    toHaveScreenshot: {
      animations: 'disabled',
      caret: 'hide',
      // Text anti-aliasing differs by a few pixels between runs of the same image; a changed
      // layout or colour is far above this.
      maxDiffPixelRatio: 0.002,
    },
  },
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
  ],
});
