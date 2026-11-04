// A theme's loop that never ends and names no rate (docs/themes.md, Drawing cheaply) redraws on every screen refresh
// and ignores "Pause animations when lull isn't the active window". Theme Studio and importing a theme say so, with the
// guide's rate and an Add it that writes it (web/designs/loops.js); then lull paces the loop, measured as the loops gate
// does: stepped to the rate it names, and held while the page asks loops to hold still. A hint, not a block.

import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

// falling petals: two pseudo-elements on the page, drifting down forever
const PETALS = `/* falling petals */
body::before, body::after { content: ""; position: fixed; z-index: 9; top: -5vh; left: 30%; width: 14px; height: 10px;
  border-radius: 70% 0; background: #f4a7c0; pointer-events: none; animation: petal-fall 9s linear infinite; }
body::after { left: 70%; animation-duration: 13s; }
@keyframes petal-fall { to { transform: translate(-6vw, 110vh) rotate(320deg); } }
`;
// the petals' loops in a page: the rate each names, whether it is stepped, its playback rate (0 = held)
const petals = (frame) => frame.evaluate(() => document.getAnimations().filter((a) => a.animationName === 'petal-fall').map((a) => {
  const e = a.effect, cs = getComputedStyle(e.target, e.pseudoElement);
  return { fps: parseFloat(cs.getPropertyValue('--loop-fps')) || 0, stepped: /steps/.test(e.getTiming().easing), rate: a.playbackRate };
}));

test('Theme Studio names a loop with no rate and what it costs, adds the rate, and the preview paces the loop', async ({ page }, info) => {
  await openScreen(page, '/settings/appearance', { clock: false });
  await page.getByRole('button', { name: 'Open Theme Studio' }).click();
  const list = page.locator('[data-studio-loops]');
  await expect(list).toContainText('Every loop that never ends names its rate');

  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill(PETALS);
  await expect(list).toContainText('petal-fall on body::before, body::after, line 3.');
  await expect(page.locator('[data-studio-loops-cost]')).toHaveText(/^It never ends and names no rate, so it redraws on every screen refresh, up to 165 times a second/);
  await expect(list).toContainText('It moves, so 20 a second suits it.');
  await list.locator('xpath=..').screenshot({ path: info.outputPath('studio-loops-before.png') });

  // the preview runs it at the screen's rate: no rate, not stepped, not held
  const frame = page.frames().find((f) => /index\.html#\/home\?.*theme=day/.test(f.url()));
  await expect.poll(() => petals(frame).then((p) => p.length)).toBe(2);
  expect(await petals(frame)).toEqual([{ fps: 0, stepped: false, rate: 1 }, { fps: 0, stepped: false, rate: 1 }]);

  await page.getByRole('button', { name: 'Add --loop-fps: 20 to body::before, body::after' }).click();
  await expect(page.locator('[data-studio="css"]')).toHaveValue(/animation: petal-fall 9s linear infinite; --loop-fps: 20; \}/);
  await expect(list).toContainText('Every loop that never ends names its rate');
  await expect(page.locator('[data-studio-loops-cost]')).toBeHidden();
  await list.locator('xpath=..').screenshot({ path: info.outputPath('studio-loops-after.png') });

  // paced: stepped to 20 a second, and held while the page asks
  await expect.poll(() => petals(frame), { timeout: 5000 }).toEqual([{ fps: 20, stepped: true, rate: 1 }, { fps: 20, stepped: true, rate: 1 }]);
  await frame.evaluate(() => App.still('gate', true));
  expect((await petals(frame)).map((p) => p.rate)).toEqual([0, 0]);
  await frame.evaluate(() => App.still('gate', false));
  expect((await petals(frame)).map((p) => p.rate)).toEqual([1, 1]);
});

test('an imported theme with a loop that names no rate is offered it, and works as it is without', async ({ page }, info) => {
  // the hub's side of an import (the fixture hub serves themes but doesn't store them): the theme as stored, and its edit
  const stored = { css: PETALS, puts: 0 };
  const item = { id: 'petals', name: 'Petals', api: 1, extends: 'lull', shell: 'rail', modes: ['day', 'night'], builtIn: false };
  await page.route('**/api/themes', (r) => (r.request().method() === 'POST' ? r.fulfill({ json: item }) : r.fallback()));
  await page.route('**/api/themes/petals/theme.css', (r) => {
    if (r.request().method() !== 'PUT') return r.fulfill({ body: stored.css, contentType: 'text/css' });
    stored.css = r.request().postData(); stored.puts++;
    return r.fulfill({ json: item });
  });
  await openScreen(page, '/settings/appearance', { clock: false });
  const drop = { name: 'petals.lulltheme', mimeType: 'application/zip', buffer: Buffer.from('PK') };

  await page.locator('[data-theme-file]').setInputFiles(drop);
  const dialog = page.getByRole('dialog', { name: 'A loop in Petals could draw less' });
  await expect(dialog).toContainText('petal-fall on body::before, body::after: It moves, so 20 a second suits it.');
  await expect(dialog).toContainText('Petals works as it is');
  await dialog.screenshot({ path: info.outputPath('import-loops.png') });
  await dialog.getByRole('button', { name: 'Not now' }).click();
  await expect(dialog).toBeHidden();
  expect(stored.puts, 'Not now leaves the theme as it is').toBe(0);

  await page.locator('[data-theme-file]').setInputFiles(drop);
  await page.getByRole('dialog', { name: 'A loop in Petals could draw less' }).getByRole('button', { name: 'Add it' }).click();
  await expect(page.locator('.toast').filter({ hasText: 'Added its rate to Petals' })).toBeVisible();
  expect(stored.css).toBe(PETALS.replace('linear infinite; }', 'linear infinite; --loop-fps: 20; }'));
});
