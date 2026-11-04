// A page part still coming shows its skeleton until the hub's push says it has arrived or failed (`artist.updated`,
// api.md), and the page must not hang on that one push: live, an artist's discography stayed a skeleton for good.
// - The push missed (the stream dropped, the app asleep): a reconnect, or the page being seen again, reads it again.
// - A part that failed says so with its retry, never an endless skeleton.

import { expect, test } from '@playwright/test';
import { ids, openScreen } from './lib.mjs';

/** This test's own scope on the fixture hub (X-Fixture-Client): its late pages and events are its own. */
async function own(page, request) {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  return {
    late: (data) => request.post('/__fixture/artist-late', { data: { id: ids.artist, ...data }, headers }),
    arrived: () => request.post('/__fixture/artist-in', { data: { id: ids.artist }, headers }),
    dropEvents: () => request.post('/__fixture/drop-events', { headers }),
  };
}

const disco = (page) => page.locator('.lib-disco');

test('a discography whose push was missed comes in when the event stream connects again', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.late({ pending: ['albums'] });
  await openScreen(page, `/artist/${ids.artist}`);
  await expect(disco(page).locator('.lib-skel-shelf')).toBeVisible();
  await hub.arrived();   // in on the hub, and the push lost
  await hub.dropEvents();
  await expect(disco(page).locator('[data-album-id]').first()).toBeVisible({ timeout: 15_000 });
  await expect(disco(page).locator('.lib-skel-shelf')).toHaveCount(0);
});

test('a discography whose push was missed comes in when the page is seen again', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.late({ pending: ['albums'] });
  await openScreen(page, `/artist/${ids.artist}`);
  await expect(disco(page).locator('.lib-skel-shelf')).toBeVisible();
  await hub.arrived();
  await page.evaluate(() => document.dispatchEvent(new Event('visibilitychange')));   // back from the background
  await expect(disco(page).locator('[data-album-id]').first()).toBeVisible();
});

test('a discography that failed says so with a retry, never a skeleton', async ({ page, request }) => {
  const hub = await own(page, request);
  await hub.late({ errors: { albums: "iTunes didn't answer within 90 s" } });
  await openScreen(page, `/artist/${ids.artist}`);
  await expect(disco(page).locator('.lib-skel-shelf')).toHaveCount(0);
  await expect(disco(page)).toContainText("Couldn't load this part");
  await expect(disco(page)).toContainText("iTunes didn't answer within 90 s");
  await expect(disco(page).getByRole('button', { name: 'Try again' })).toBeVisible();
});
