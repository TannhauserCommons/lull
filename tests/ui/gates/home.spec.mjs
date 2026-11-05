// Home's parts the hub fills for the listener it knows (docs/context/discovery.md#home-feed):
// - a first run (the feed's `cold`) offers tags and the artists of its starter picks to follow;
// - each followed tag brings the tags that come with it ("you might like"), whatever pages were opened before;
// - a pick's "why" says what the listener did (the reason's evidence), never more.

import { expect, test } from '@playwright/test';
import { openScreen } from './lib.mjs';

/** This test's own feed on the fixture hub (X-Fixture-Client). */
async function feedOf(page, request, shape) {
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await page.context().setExtraHTTPHeaders(headers);
  await request.post('/__fixture/feed-shape', { data: { shape }, headers });
}

test('a first run offers tags and the artists of its starter picks', async ({ page, request }) => {
  await feedOf(page, request, 'cold');
  await openScreen(page, '/home');
  const cold = page.locator('.disc-cold');
  await expect(cold).toContainText('pick a few to start');
  await expect(cold.locator('.disc-cloud__tag').first()).toBeVisible();
  await expect(cold).toContainText('or a few artists');
  await expect(cold.locator('.disc-cold__artist')).toHaveCount(6);
});

test('a followed tag brings the tags that come with it, with no tag page opened', async ({ page, request }) => {
  await feedOf(page, request, 'related');
  await openScreen(page, '/home');
  const tags = page.locator('[data-region="tags"]');
  await expect(tags).toContainText('you might like');
  await expect(tags.locator('.cluster .chip').first()).toBeVisible();
});

test('a pick says why from what the listener did', async ({ page, request }) => {
  await feedOf(page, request, 'related');
  await openScreen(page, '/home');
  const feed = await page.evaluate(() => App.data.api('/api/feed'));
  const pick = feed.picks.find((p) => p.reasons[0].evidence);
  await page.locator(`[data-region="picks"] .chip-reason[data-item="${pick.trackId}"]`).click();
  await expect(page.locator('.disc-why')).toContainText(`Similar to ${pick.reasons[0].name}. You've played them 14 times.`);
});
