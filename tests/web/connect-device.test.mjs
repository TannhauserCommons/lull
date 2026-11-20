// What makes a browser one device in lull Connect (web/connect.js): a device is online when the hub says so and not
// because a state arrived; one tab of the browser acts as the device; and what a device reports is what "Play here"
// needs to play its queue as it was.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { browserLocks, connectPage, settle } from './connect-page.mjs';

const kitchen = { id: 'd1', name: 'Kitchen', kind: 'browser', online: false };
const state = { trackId: 'a', queue: ['a'], queueIndex: 0, position: 3, playing: true };

test('a state from a device does not make it online; the hub\'s word does', async () => {
  const page = connectPage({ items: [{ ...kitchen }] });
  await page.start();
  page.emit('server:device.state', { deviceId: 'd1', ...state });
  assert.deepEqual(page.App.connect.others(), [], 'a device the hub lists offline was shown online by a state');
  page.emit('server:device.online', { deviceId: 'd1', online: true });
  assert.deepEqual(page.App.connect.others().map((d) => d.id), ['d1']);
  page.emit('server:device.online', { deviceId: 'd1', online: false });
  assert.deepEqual(page.App.connect.others(), []);
});

test('only one tab of a browser obeys a command for the device', async () => {
  const locks = browserLocks(), tabs = [connectPage({ locks: locks.tab() }), connectPage({ locks: locks.tab() })];
  for (const tab of tabs) await tab.start();
  for (const tab of tabs) tab.emit('server:device.command', { deviceId: 'me', action: 'pause', from: 'd1' });
  assert.deepEqual(tabs.map((t) => t.asked.length), [1, 0], 'both tabs paused');
});

test('only one tab of a browser reports for the device, the others stay out of its state', async () => {
  const locks = browserLocks(), tabs = [connectPage({ locks: locks.tab() }), connectPage({ locks: locks.tab() })];
  for (const tab of tabs) await tab.start();
  for (const tab of tabs) tab.emit('player', 'track');
  assert.ok(tabs[0].reports().length > 0);
  assert.equal(tabs[1].reports().length, 0, 'the second tab reported over the first');
});

test('when the acting tab closes, a tab that waited takes the device on and reports at once', async () => {
  const locks = browserLocks(), a = locks.tab(), b = locks.tab();
  const tabs = [connectPage({ locks: a }), connectPage({ locks: b })];
  for (const tab of tabs) await tab.start();
  a.close(); await settle();
  assert.equal(tabs[1].reports().length, 1, 'the new acting tab didn\'t report');
  tabs[1].emit('server:device.command', { deviceId: 'me', action: 'next', from: 'd1' });
  assert.equal(tabs[1].asked.length, 1);
});

test('a tab where the listener starts playing becomes the device; the tab it took it from acts again once that one closes', async () => {
  const locks = browserLocks(), a = locks.tab(), b = locks.tab();
  const tabs = [connectPage({ locks: a }), connectPage({ locks: b })];
  for (const tab of tabs) await tab.start();
  tabs[1].player.state.playing = true;
  tabs[1].emit('player', 'play'); await settle();
  for (const tab of tabs) tab.emit('server:device.command', { deviceId: 'me', action: 'pause', from: 'd1' });
  assert.deepEqual(tabs.map((t) => t.asked.length), [0, 1]);
  b.close(); await settle();
  tabs[0].emit('server:device.command', { deviceId: 'me', action: 'pause', from: 'd1' });
  assert.equal(tabs[0].asked.length, 1);
});

test('a browser without Web Locks acts in every tab, as before', async () => {
  const tabs = [connectPage(), connectPage()];
  for (const tab of tabs) await tab.start();
  for (const tab of tabs) tab.emit('server:device.command', { deviceId: 'me', action: 'pause', from: 'd1' });
  assert.deepEqual(tabs.map((t) => t.asked.length), [1, 1]);
});

test('the state a device reports carries its modes, each entry\'s group and place before shuffling, and its radio', async () => {
  const page = connectPage({ known: ['a', 'b'] });
  await page.start();
  const s = page.player.state;
  s.queue = [{ trackId: 'a', group: 'context', orig: 1 }, { trackId: 'b', group: 'radio' }];
  s.index = 0; s.shuffle = true; s.repeat = 'one';
  s.radio = { name: 'Sade', seed: { kind: 'artist', id: 'ar-sade' }, keepsAdding: true, steer: 0.4, looking: true, error: 'no' };
  page.emit('player', 'track');
  const sent = page.reports().pop();
  assert.deepEqual(sent.queue, ['a', 'b']);
  assert.deepEqual([sent.shuffle, sent.repeat, sent.groups, sent.order], [true, 'one', ['context', 'radio'], [1, null]]);
  assert.deepEqual(sent.radio, { name: 'Sade', seed: { kind: 'artist', id: 'ar-sade' }, keepsAdding: true, steer: 0.4 });
  s.radio = null;
  s.shuffle = false; page.emit('player', 'mode');
  await settle();
  assert.equal(page.reports().length, 2, 'a change of mode wasn\'t reported');
});

test('a report the hub refused because no stream was open is sent again when the hub says the device is online', async () => {
  let refuse = true;
  const page = connectPage({ post: () => (refuse ? Promise.reject(Object.assign(new Error('That device isn\'t connected.'), { status: 409 })) : Promise.resolve({})) });
  await page.start();
  assert.equal(page.reports().length, 1);
  refuse = false;
  page.emit('server:device.online', { deviceId: 'me', online: true });
  await settle();
  assert.equal(page.reports().length, 2);
});

test('a reconnected event stream rereads the device list, which may have missed a device coming online, and reports again', async () => {
  const items = [{ ...kitchen }];
  const page = connectPage({ items });
  await page.start();
  assert.deepEqual(page.App.connect.others(), []);
  items[0].online = true;                               // came online while this page's stream was down
  page.App.connect.resume(); await settle();
  assert.deepEqual(page.App.connect.others().map((d) => d.id), ['d1']);
  assert.equal(page.reports().length, 2);
});
