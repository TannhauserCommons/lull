// "Play here" (web/connect.js C.takeover): the other device's whole queue continues on this one at the entry it was
// playing, even when that song is in the queue twice, and when songs this device can't read are left out of the list
// before it; its shuffle, repeat, groups and radio come with it.
// connect.js runs in a small stand-in page (connect-page.mjs), with the player's adopt recorded.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { connectPage } from './connect-page.mjs';

/* Connect on a page that knows `known`; the other device ("d1") reports `state` (a queue playing the entry at `queueIndex`) */
async function takeover(known, queue, queueIndex, more = {}) {
  const device = { id: 'd1', name: 'Kitchen', state: { trackId: queue[queueIndex], queue, queueIndex, position: 12, playing: true, recv: Date.now(), ...more } };
  const page = connectPage({ known });
  page.store.devices = { items: [device], me: 'me' };   // after connect.js has set its own empty list
  const done = page.App.connect.takeover(device);
  page.emit('server:device.state', { deviceId: 'd1', ...device.state });
  await done;
  const [saved, playing] = page.adopted[0] || [];
  return { page, adopted: page.adopted, saved, playing, toasts: page.toasts, ids: saved && saved.queue.map((q) => q.trackId) };
}

test('Play here starts at the entry the other device was on, not the first copy of that song', async () => {
  const { adopted, ids, saved } = await takeover(['a', 'b', 'c'], ['a', 'b', 'a', 'c'], 2);
  assert.equal(adopted.length, 1);
  assert.deepEqual(ids, ['a', 'b', 'a', 'c']);
  assert.equal(saved.index, 2);
});

test('Play here leaves out the songs this device can\'t read and still starts at the entry the other device was on', async () => {
  const { ids, saved } = await takeover(['a', 'b', 'c'], ['x', 'a', 'y', 'b', 'a', 'c'], 4);
  assert.deepEqual(ids, ['a', 'b', 'a', 'c']);
  assert.equal(saved.index, 2);
});

test('Play here says so when the song the other device was playing can\'t be read', async () => {
  const { adopted, toasts } = await takeover(['a', 'b'], ['a', 'x', 'b'], 1);
  assert.equal(adopted.length, 0);
  assert.match(toasts[0][0], /Couldn't read the song Kitchen was playing/);
});

test('Play here carries the whole queue, not a stretch of it around the song', async () => {
  const queue = Array.from({ length: 600 }, (_, i) => `t${i}`);
  const { ids, saved } = await takeover(queue, queue, 300);
  assert.equal(ids.length, 600);
  assert.equal(saved.index, 300);
  assert.ok(Math.abs(saved.pos - 12) < 1, 'plays on from the position the other device reported');
});

test('Play here carries shuffle, repeat, each entry\'s group and place before shuffling, and the radio', async () => {
  const radio = { name: 'Sade', seed: { kind: 'artist', id: 'ar-sade' }, keepsAdding: true, steer: 0.4 };
  const { saved, playing } = await takeover(['a', 'b', 'c', 'd'], ['a', 'x', 'b', 'c', 'd'], 2, {
    shuffle: true, repeat: 'all', groups: ['context', 'next', 'next', 'context', 'radio'], order: [3, 9, null, 0, 1], radio,
  });
  assert.deepEqual(saved.queue.map((q) => [q.trackId, q.group, q.orig]), [['a', 'context', 3], ['b', 'next', null], ['c', 'context', 0], ['d', 'radio', 1]]);
  assert.equal(saved.index, 1);
  assert.equal(saved.shuffle, true);
  assert.equal(saved.repeat, 'all');
  assert.deepEqual(saved.radio, radio);
  assert.equal(playing, true);
});

test('Play here from a device that reports no modes plays the queue in order, as it always did', async () => {
  const { saved } = await takeover(['a', 'b'], ['a', 'b'], 0);
  assert.deepEqual(saved.queue.map((q) => q.group), ['context', 'context']);
  assert.ok(!saved.shuffle && !saved.radio);
  assert.ok(!saved.repeat || saved.repeat === 'off');
});
