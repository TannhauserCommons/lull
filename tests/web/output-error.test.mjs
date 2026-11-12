// What the player does when its audio fails (LullRules.afterOutputError, web/player-rules.js). Live (2026-11-12): a
// copy of TLC's "No Scrubs" picked the day before stayed with its queue entry, so every later play asked the hub for
// that copy; when it failed, the song stopped on "this copy can't be played", and Retry asked for the same copy again.
// A pick that can't play gives way to lull's own choice; only that choice failing is the song's outcome.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const R = require('../../web/player-rules.js');

test('a copy the listener picked that fails gives way to lull\'s own choice', () => {
  assert.equal(R.afterOutputError({ resuming: false, soulseek: true, picked: true }), 'unpick');
});

test('lull\'s own choice failing asks the hub for the song\'s outcome', () => {
  assert.equal(R.afterOutputError({ resuming: false, soulseek: true, picked: false }), 'ask-hub');
});

test('a song restored after a reload starts again from 0:00 first, then its pick gives way', () => {
  assert.equal(R.afterOutputError({ resuming: true, soulseek: true, picked: true }), 'restart');
  assert.equal(R.afterOutputError({ resuming: false, soulseek: true, picked: true }), 'unpick');
});

test('a saved song or a downloaded file says its own reason', () => {
  assert.equal(R.afterOutputError({ resuming: false, soulseek: false, picked: false }), 'own');
  assert.equal(R.afterOutputError({ resuming: false, soulseek: false, picked: true }), 'own');
});
