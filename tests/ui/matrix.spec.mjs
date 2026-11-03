// Every route x design x theme, on the phone and desktop projects (playwright.config.mjs): each
// page must settle without console errors, failed requests or sideways scrolling, and match its
// golden screenshot. One test per picture, so a failure names the exact route, design and theme.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test } from '@playwright/test';
import { overflow, settle, squeezed, watch } from './checks.mjs';
import { designs, detailRoutes, fixedRoutes, slug, themes } from './routes.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const { failures: tolerated } = JSON.parse(readFileSync(join(here, 'expected-failures.json'), 'utf8'));
// A fixed "now", so relative times ("3 minutes ago") and date ranges read the same on every run.
const NOW = new Date('2026-11-01T12:00:00Z');

for (const route of [...fixedRoutes, ...detailRoutes()]) {
  for (const design of designs) {
    for (const theme of themes) {
      test(`${slug(route)} / ${design} / ${theme}`, async ({ page }) => {
        const problems = watch(page, tolerated);
        await page.clock.setFixedTime(NOW);
        // The stored preferences the UI reads before first paint (index.html): design and theme.
        await page.addInitScript(
          ([d, t]) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: t })),
          [design, theme],
        );
        await page.goto(`/#${route}`);
        await settle(page, problems);

        expect.soft(await overflow(page), 'content wider than the viewport').toBeNull();
        expect.soft(await squeezed(page), 'a title squeezed to a few letters').toBeNull();
        expect.soft(problems.console, 'console errors').toEqual([]);
        expect.soft(problems.requests, 'failed requests').toEqual([]);
        await expect(page).toHaveScreenshot(`${slug(route)}-${design}-${theme}.png`);
      });
    }
  }
}
