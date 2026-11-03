// axe-core on the key screens in every design and theme (WCAG 2.2 A and AA, contrast included). A violation fails
// unless a11y-known.json lists it with the reason; that file only shrinks: fix a listed one and delete its entry.

import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import AxeBuilder from '@axe-core/playwright';
import { expect, test } from '@playwright/test';
import { settle, watch } from '../checks.mjs';
import { designs, NOW, screens, themes } from './lib.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const known = JSON.parse(readFileSync(join(here, 'a11y-known.json'), 'utf8')).known;

for (const [name, route] of Object.entries(screens)) {
  for (const design of designs) {
    for (const theme of themes) {
      test(`${name} / ${design} / ${theme}`, async ({ page }) => {
        const problems = watch(page);
        await page.clock.setFixedTime(NOW);
        await page.addInitScript(([d, t]) => localStorage.setItem('lull:prefs', JSON.stringify({ design: d, theme: t, motion: 'reduce' })), [design, theme]);
        await page.goto(`/#${route}`);
        await settle(page, problems);
        const { violations } = await new AxeBuilder({ page }).withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
        const found = violations.flatMap((v) =>
          v.nodes
            .map((n) => ({ rule: v.id, target: n.target.join(' '), summary: n.failureSummary?.split('\n')[1]?.trim() }))
            .filter((x) => !known.some((k) => k.rule === x.rule && (!k.designs || k.designs.includes(design)) && new RegExp(k.target).test(x.target))),
        );
        expect(found, `${found.length} accessibility violation(s)`).toEqual([]);
      });
    }
  }
}
