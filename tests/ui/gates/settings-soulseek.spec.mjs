// Settings › Sources signs a hub that runs its own slskd (the desktop app's, the Linux package's) in to Soulseek from
// nothing: one form takes the user name and the password and sends them together, as the hub takes them only together
// (crates/core's Config refuses a name without a password). Two fields saved one at a time used to send the name alone,
// and the listener saw "LULL_SOULSEEK_USERNAME and LULL_SOULSEEK_PASSWORD must be set together". On a fixture hub of its
// own, with FIXTURE_HUB_SLSKD=1 (a managed slskd, no account yet).

import { expect, test } from '@playwright/test';
import { openScreen, ownHub } from './lib.mjs';

let hub;
test.beforeAll(async () => { hub = await ownHub({ FIXTURE_HUB_SLSKD: '1' }); });
test.afterAll(() => hub?.close());

test('a Soulseek account goes in from Settings in one go, with no key names in sight', async ({ browser }) => {
  const context = await browser.newContext({ baseURL: hub.url, viewport: { width: 1440, height: 900 } });
  const page = await context.newPage();
  const sent = [];
  page.on('request', (r) => { if (r.url().endsWith('/api/config') && r.method() === 'PATCH') sent.push(r.postDataJSON()); });
  await openScreen(page, '/settings/sources', { clock: false });

  const form = page.locator('#view [data-form="hub-slsk"]');
  await expect(form, 'Settings › Sources shows the sign-in form while no account is set').toBeVisible();
  await form.getByLabel('User name').fill('nightowl');
  await form.getByLabel('Password').fill('a-secret');
  await form.getByRole('button', { name: 'Sign in' }).click();

  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0], 'both keys, together, in one request').toEqual({ LULL_SOULSEEK_USERNAME: 'nightowl', LULL_SOULSEEK_PASSWORD: 'a-secret' });
  await expect(page.locator('#hub-slsk')).toContainText('nightowl');
  await expect(page.locator('#hub-slsk [role="alert"]'), 'no error').toHaveCount(0);
  await expect(page.locator('#view'), 'no internal key names').not.toContainText('LULL_SOULSEEK');
  await context.close();
});
