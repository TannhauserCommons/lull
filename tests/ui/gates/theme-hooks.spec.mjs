// A theme that styles hooks lull moved, removed or keeps internal is told so in plain words, at import and in Theme
// Studio, and still imports (designs/hooks.js, docs/themes.md section 9, Hooks that changed): a warning, never a
// refusal. On a fixture hub of the gate's own.

import { expect, test } from '@playwright/test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { crc32 } from 'node:zlib';
import { openScreen, ownHub } from './lib.mjs';

// the player bar as a theme placed it before Like and Save moved; a class lull never had; one of lull's internal parts
const CSS = `@media (min-width: 1100px) {
  .pbar { grid-template-areas: "now like ctl lcd save"; }
  .pbar [data-act="like"] { grid-area: like; }
  .pbar [data-act="save"] { grid-area: save; }
}
.lib-row .btn-icon { opacity: .8; }
.lib-title { letter-spacing: .02em; }
.wired-mark { color: #ffa53d; }
`;
const theme = {
  'theme.json': JSON.stringify({ id: 'wired', name: 'Wired', api: 1, extends: null, shell: 'rail', swatches: ['#141516', '#d6d3c9', '#ffa53d', '#ffb24a'], modes: ['day', 'night'] }),
  'theme.css': CSS,
  'slots/brand.html': '<span class="wordmark wired-mark">{{product}}</span>',
};

let hub, dir;
test.beforeAll(async () => { dir = mkdtempSync(join(tmpdir(), 'lull-theme-hooks-')); hub = await ownHub({ FIXTURE_THEMES: dir }); });
test.afterAll(() => { hub?.close(); rmSync(dir, { recursive: true, force: true }); });

test('a theme styling hooks lull moved, removed or keeps internal imports, and is told which and what to use instead', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await openScreen(page, '/settings/appearance', { clock: false });
  await page.locator('[data-theme-file]').setInputFiles({ name: 'wired.lulltheme', mimeType: 'application/zip', buffer: zip(theme) });

  const dialog = page.getByRole('dialog', { name: /Wired styles parts lull has changed/ });
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText('Like and Save moved next to the song, into .pbar__acts');
  await expect(dialog).toContainText('To move both, place .pbar__acts.');
  await expect(dialog.locator('li', { hasText: '.lib-row' })).toContainText('Not in lull, or in this theme’s own slots');
  await expect(dialog.locator('li', { hasText: '.lib-title' })).toContainText('One of lull’s internal parts');
  await expect(dialog, 'its own slots\' class is its own').not.toContainText('.wired-mark');
  await dialog.getByRole('button', { name: 'OK' }).click();
  await expect(page.locator('.sys-design[data-value="wired"]'), 'it imported all the same').toBeVisible();

  await page.locator('[data-act="theme-edit"][data-id="wired"]').click();
  const hooks = page.locator('[data-studio-hooks]');
  await expect(hooks).toContainText('Like and Save in the player bar');
  await expect(hooks).toContainText('line 3');
  await expect(hooks).toContainText('line 6');

  // fixed and saved, the check says so
  await page.locator('[data-studio-tab="css"]').click();
  await page.locator('[data-studio="css"]').fill('.wired-mark { color: #ffa53d; }\n.pbar__acts { margin-left: 6px; }\n');
  await expect(hooks, 'not while typing').toContainText('Like and Save in the player bar');
  await page.locator('[data-studio-act="save"]').click();
  await expect(hooks).toContainText('Every class and data-* attribute it styles is one lull or its own slots have');
  await context.close();
});

/* a .lulltheme: the files, stored (a zip without compression) */
function zip(files) {
  const local = [], central = [];
  let at = 0;
  for (const [name, text] of Object.entries(files)) {
    const data = Buffer.from(text), path = Buffer.from(name), crc = crc32(data);
    const head = Buffer.alloc(30);
    head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt32LE(crc, 14);
    head.writeUInt32LE(data.length, 18); head.writeUInt32LE(data.length, 22); head.writeUInt16LE(path.length, 26);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0); dir.writeUInt16LE(20, 4); dir.writeUInt16LE(20, 6); dir.writeUInt32LE(crc, 16);
    dir.writeUInt32LE(data.length, 20); dir.writeUInt32LE(data.length, 24); dir.writeUInt16LE(path.length, 28); dir.writeUInt32LE(at, 42);
    local.push(head, path, data); central.push(dir, path);
    at += 30 + path.length + data.length;
  }
  const size = central.reduce((n, b) => n + b.length, 0), end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(central.length / 2, 8); end.writeUInt16LE(central.length / 2, 10);
  end.writeUInt32LE(size, 12); end.writeUInt32LE(at, 16);
  return Buffer.concat([...local, ...central, end]);
}
