// A seek past what has arrived is kept while the song plays on (web/app.js, P.seekFraction), and it is over once the song
// has played up to it. The owner (2026-11-08, a FLAC from Soulseek): the state tab still said "Waiting for 2:29" at 2:32.
// The player had stopped hearing how far the copy had arrived (the hub's two fetches of one song, fixed in soulseek's
// Fetcher::stream), and only what arrived landed a kept seek; the song playing past it is proof enough.
// docs/context/quality.md#ui-gates.

import { expect, test } from '@playwright/test';
import { fakeOutput, openScreen } from './lib.mjs';

const fixtureClient = () => `${test.info().testId}:${test.info().repeatEachIndex}`;

test('a kept seek is over once the song has played up to it', async ({ page, request }) => {
  await page.context().setExtraHTTPHeaders({ 'x-fixture-client': fixtureClient() });
  await page.addInitScript(fakeOutput);
  await openScreen(page, '/home', { clock: false });
  const [id] = await page.evaluate(() => App.data.api('/__fixture/unsaved-songs?n=1').then((r) => { App.data.absorb(r.entities); return r.items; }));
  await page.evaluate((id) => App.player.playList([id], 0), id);
  await page.waitForFunction(() => App.player.state.playing && App.player.state.pos > 0.2);
  // a tenth has arrived, at no known speed, from a copy the hub can't start further on: a seek to the middle waits
  const size = 30_000_000;
  await request.post('/__fixture/emit', { headers: { 'x-fixture-client': fixtureClient() }, data: { topic: 'stream', data: {
    trackId: id, jobId: 'flac-job', status: 'streaming', attempt: 1, peer: 'quietcanal', format: 'FLAC 16/44.1', youtube: false,
    received: size / 10, size, ranges: [[0, 0.1]], speed: 0, seekAheadS: null, failedPeers: [], peersFound: 1, belowMinimum: false, unverified: false } } });
  await page.waitForFunction(() => App.player.state.peer && App.player.state.peer.name === 'quietcanal');
  const middle = await page.evaluate(() => { App.player.seekFraction(0.5); return 0.5 * App.player.duration(); });
  expect(await page.evaluate(() => App.player.seekWaitText())).toMatch(/^Waiting for \d+:\d\d$/);

  // short of it, it still waits
  await page.evaluate((s) => window.__out.at(s), middle - 5);
  await page.waitForFunction((s) => App.player.state.pos >= s, middle - 5);
  await page.waitForTimeout(500);
  expect(await page.evaluate(() => App.player.state.seekWait), 'kept until the song gets there').not.toBeNull();

  // the song plays on past it: nothing left to wait for, and no jump
  await page.evaluate((s) => window.__out.at(s), middle + 1);
  await expect.poll(() => page.evaluate(() => App.player.state.seekWait)).toBeNull();
  expect(await page.evaluate(() => App.player.seekWaitText())).toBeNull();
  await expect(page.locator('#pbar')).not.toContainText('Waiting for');
  expect(await page.evaluate(() => window.__out.seeks), 'the song was never moved').toEqual([]);
});
