// Home's parts the hub fills for the listener it knows (docs/context/discovery.md#home-feed):
// - a first run (the feed's `cold`) offers tags and the artists of its starter picks to follow;
// - each followed tag brings the tags that come with it ("you might like"), whatever pages were opened before;
// - a pick's "why" says what the listener did (the reason's evidence), never more;
// - Refresh picks builds a new shelf on the hub and waits for it, and says when the next one can be made (hourly);
// - each pick says quietly where it can play from, as the hub's checks come in (`feed.avail`), and one with no copy
//   anywhere offers "Search again";
// - Home's lead follows the player: a song started on Home shows in it at once, in every design.

import { expect, test } from '@playwright/test';
import { designs, openScreen } from './lib.mjs';

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

test('Refresh picks builds a new shelf and waits for it; again within the hour says when', async ({ page, request }) => {
  await feedOf(page, request, 'related');
  await openScreen(page, '/home');
  const picks = page.locator('[data-region="picks"]');
  const asked = page.waitForRequest((r) => r.url().endsWith('/api/feed/refresh') && r.method() === 'POST');
  await picks.getByRole('button', { name: 'Refresh picks' }).click();
  await asked;
  await expect(picks.locator('.disc-pick').first()).toBeVisible();      // back once feed.updated brought the feed
  await picks.getByRole('button', { name: 'Refresh picks' }).click();
  await expect(page.locator('.toast', { hasText: /Fresh ones in (an hour|\d+ minutes)/ })).toBeVisible();
});

test('a pick says where it can play from, and one with no copy offers Search again', async ({ page, request }) => {
  await feedOf(page, request, 'avail');
  await openScreen(page, '/home');
  const picks = page.locator('[data-region="picks"] .disc-pick');
  await expect(picks.nth(0)).toHaveAttribute('data-avail', 'soulseek');
  await expect(picks.nth(1).locator('.pick-avail')).toHaveAttribute('aria-label', 'On Soulseek and YouTube');
  await expect(picks.nth(2).locator('.pick-avail__src[data-src="youtube"]')).toBeVisible();
  const none = page.locator('[data-region="picks"] .disc-pick.is-nosource');      // listed after the playable ones
  await expect(none).toHaveCount(1);
  await expect(none.getByRole('button', { name: 'Search again' })).toBeVisible();
  // a check answered after Home drew the shelf
  const id = await picks.nth(3).getAttribute('data-track-id');
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await request.post('/__fixture/emit', { data: { topic: 'feed.avail', data: { trackId: id, avail: ['soulseek'], noSource: false } }, headers });
  await expect(page.locator(`[data-region="picks"] .disc-pick[data-track-id="${id}"]`)).toHaveAttribute('data-avail', 'soulseek');
});

test('a slow connection still opens the why popover at once, with the full sentence once it is in', async ({ page, request }) => {
  await feedOf(page, request, 'related');
  // the sentences' code held back 2 s: the popover must not wait for it
  await page.route('**/screens/why.js*', async (route) => { await new Promise((r) => setTimeout(r, 2000)); await route.continue(); });
  await openScreen(page, '/home');
  const feed = await page.evaluate(() => App.data.api('/api/feed'));
  const pick = feed.picks.find((p) => p.reasons[0].evidence);
  const opened = Date.now();
  await page.locator(`[data-region="picks"] .chip-reason[data-item="${pick.trackId}"]`).click();
  await expect(page.locator('.disc-why')).toContainText(pick.reasons[0].name, { timeout: 500 });
  expect(Date.now() - opened).toBeLessThan(1500);
  await expect(page.locator('.disc-why')).toContainText("You've played them 14 times.", { timeout: 5000 });
});

// Home's lead (a design's homeLead slot: Glasshouse's media window, Starlight's sky) is drawn from the player, so a song
// started while Home is open shows in it at once, as the next visit to Home draws it (in Glasshouse it appears: with
// nothing playing it draws nothing), in every design
for (const design of designs) {
  test(`${design}: a song started on Home shows in Home's lead at once`, async ({ page, request }) => {
    await feedOf(page, request, 'related');
    await openScreen(page, '/home', { design });
    const drawn = () => page.locator('#view .disc-home').evaluate((h) => h.querySelector('[data-region="lead"]')?.innerHTML ?? 'none');
    const pick = page.locator('[data-region="picks"] .disc-pick:not(.is-nosource)').first();
    const title = await pick.locator('.disc-pick__title').textContent();
    await pick.hover();
    await pick.locator('.disc-pick__play').click();
    await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.3);
    await page.evaluate(() => App.player.pause());   // the clock still: the lead's time reads the same twice
    const live = await drawn();
    if (design === 'glasshouse') expect(live, "Glasshouse's media window shows the song").toContain(title);
    await page.evaluate(() => App.refresh());
    expect(live, 'the lead as Home draws it afresh').toBe(await drawn());
  });
}
