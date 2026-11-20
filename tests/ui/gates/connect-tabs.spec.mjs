// Two tabs of one browser are one device in lull Connect (connect.js, "one tab acts"): a command for the device is
// obeyed by one of them, and by the other once that one closes. The browser's own Web Locks decide, not a stand-in.

import { expect, test } from '@playwright/test';
import { openScreen, screens } from './lib.mjs';

test('a command for this device is obeyed by one tab of the browser, and by the next when it closes', async ({ context, request }) => {
  // This test's own events on the fixture hub (server.mjs, X-Fixture-Client): both its tabs hear them, as a hub's would.
  const headers = { 'x-fixture-client': `${test.info().testId}:${test.info().repeatEachIndex}` };
  await context.setExtraHTTPHeaders(headers);
  const tabs = [await context.newPage(), await context.newPage()];
  for (const tab of tabs) {
    await openScreen(tab, screens.settings);
    await tab.waitForFunction(() => App.store.get('devices').me);
    await tab.evaluate(() => { window.__nexts = 0; App.player.next = () => { window.__nexts += 1; }; });
  }
  const nexts = () => Promise.all(tabs.map((tab) => (tab.isClosed() ? 0 : tab.evaluate(() => window.__nexts))));
  const sum = async () => (await nexts()).reduce((a, b) => a + b, 0);
  const command = () => request.post('/__fixture/emit', { headers, data: { topic: 'device.command', data: { deviceId: 'd-fixture', action: 'next', from: 'd-phone' } } });

  await command();
  await expect.poll(sum).toBe(1);
  await tabs[0].waitForTimeout(500);                       // the other tab heard it too, and let it be
  expect(await sum()).toBe(1);

  const acting = (await nexts()).findIndex((n) => n === 1);
  await tabs[acting].close();
  await command();
  await expect.poll(() => tabs[1 - acting].evaluate(() => window.__nexts), { message: 'the tab that waited took the device on' }).toBe(1);
});
