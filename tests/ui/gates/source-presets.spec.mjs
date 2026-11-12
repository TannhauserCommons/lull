// Source presets (ADR-0017): Settings › Sources makes, edits, renames, chooses and deletes them, and a phone uses Phone
// until it chooses. A listener on an iPhone on mobile data got FLAC, because every device streamed by the hub's one
// profile. On a fixture hub of its own, so its presets start as the built-ins and change nobody else's.

import { expect, test } from '@playwright/test';
import { openScreen, ownHub } from './lib.mjs';

const IPHONE = 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1';

let hub;
test.beforeAll(async () => { hub = await ownHub(); });
test.afterAll(() => hub?.close());

const presetsOf = async (page) => (await page.request.get(hub.url + '/api/sources/presets')).json();

test('a preset is made, edited, chosen for this device, renamed and deleted from Settings › Sources', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  await openScreen(page, '/settings/sources', { clock: false });
  const view = page.locator('#view');
  const uses = view.locator('[data-presets="device"]'), edit = view.locator('[data-presets="edit"]');

  await expect(uses.getByRole('button', { name: 'Desktop' }), 'a computer uses Desktop until it chooses').toHaveAttribute('aria-pressed', 'true');
  await expect(view).toContainText('This device counts as a computer, so it uses Desktop.');
  await expect(edit.getByRole('button', { name: 'Desktop' }), 'the rows edit the preset this device uses').toHaveAttribute('aria-pressed', 'true');
  await expect(view.getByRole('button', { name: 'Delete' }), 'a built-in is never deleted').toHaveCount(0);

  // A new preset starts as a copy of the one edited, and the rows edit it from then on.
  await view.getByRole('button', { name: 'New preset' }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name').fill('Car');
  await dialog.getByRole('button', { name: 'Make it' }).click();
  await expect(dialog).toHaveCount(0);
  await expect(edit.getByRole('button', { name: 'Car' })).toHaveAttribute('aria-pressed', 'true');
  await expect(uses.getByRole('button', { name: 'Desktop' }), 'making one chooses nothing').toHaveAttribute('aria-pressed', 'true');

  await view.getByRole('radiogroup', { name: 'Maximum bitrate' }).getByRole('radio', { name: '192' }).click();
  await expect.poll(async () => (await presetsOf(page)).presets.map((p) => [p.name, p.stream.maxKbps]))
    .toEqual([['Desktop', 0], ['Phone', 320], ['Car', 192]]);

  // Editing another preset doesn't change which one this device uses.
  await edit.getByRole('button', { name: 'Phone' }).click();
  await expect(view.getByRole('radiogroup', { name: 'Maximum bitrate' }).getByRole('radio', { name: '320' })).toHaveAttribute('aria-checked', 'true');
  await edit.getByRole('button', { name: 'Car' }).click();
  await expect(view.getByRole('radiogroup', { name: 'Maximum bitrate' }).getByRole('radio', { name: '192' })).toHaveAttribute('aria-checked', 'true');

  await uses.getByRole('button', { name: 'Car' }).click();
  await expect(uses.getByRole('button', { name: 'Car' })).toHaveAttribute('aria-pressed', 'true');
  expect((await presetsOf(page)).device).toMatchObject({ preset: 'p-1', chosen: true });
  await expect(view.getByRole('button', { name: 'Use Desktop, this device\'s default' })).toBeVisible();

  await view.getByRole('button', { name: 'Rename' }).click();
  await dialog.getByLabel('Name').fill('Phone');
  await dialog.getByRole('button', { name: 'Rename' }).click();
  await expect(dialog.getByRole('alert'), 'the hub\'s sentence, in the dialog').toContainText('already a preset called Phone');
  await dialog.getByLabel('Name').fill('Car stereo');
  await dialog.getByRole('button', { name: 'Rename' }).click();
  await expect(uses.getByRole('button', { name: 'Car stereo' })).toHaveAttribute('aria-pressed', 'true');

  await view.getByRole('button', { name: 'Delete' }).click();
  await dialog.getByRole('button', { name: 'Delete' }).click();
  await expect(uses.getByRole('button', { name: 'Car stereo' })).toHaveCount(0);
  await expect(uses.getByRole('button', { name: 'Desktop' }), 'back to its default').toHaveAttribute('aria-pressed', 'true');
  expect((await presetsOf(page)).device).toMatchObject({ preset: 'desktop', chosen: false });
  await context.close();
});

test('a phone uses Phone until it chooses, and can go back to it', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, userAgent: IPHONE });
  const page = await context.newPage();
  await openScreen(page, '/settings/sources', { clock: false });
  const view = page.locator('#view'), uses = view.locator('[data-presets="device"]');
  await expect(uses.getByRole('button', { name: 'Phone' })).toHaveAttribute('aria-pressed', 'true');
  await expect(view).toContainText('This device counts as a phone, so it uses Phone.');
  await expect(view.getByRole('radiogroup', { name: 'Maximum bitrate' }).getByRole('radio', { name: '320' }), 'Phone is capped').toHaveAttribute('aria-checked', 'true');
  await expect(view.locator('[data-formats="preset.minimum.formats"] [data-format="flac"]'), 'with FLAC off').toHaveAttribute('aria-pressed', 'false');

  await uses.getByRole('button', { name: 'Desktop' }).click();
  await expect(uses.getByRole('button', { name: 'Desktop' })).toHaveAttribute('aria-pressed', 'true');
  await view.getByRole('button', { name: 'Use Phone, this device\'s default' }).click();
  await expect(uses.getByRole('button', { name: 'Phone' })).toHaveAttribute('aria-pressed', 'true');
  await expect(view.getByRole('button', { name: 'Use Phone, this device\'s default' })).toHaveCount(0);
  await context.close();
});
