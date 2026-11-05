// The easter eggs' conditions (web/egg-rules.js, ADR-0081): any language, the device's clock, once per period.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const E = require('../../web/egg-rules.js');
const at = (y, mo, d, h = 0, mi = 0, s = 0) => new Date(y, mo - 1, d, h, mi, s);

test('words match whole words in any language, ignoring case and accents, and inside unspaced scripts', () => {
  assert.equal(E.findWord(['moon'], 'Fly Me to the Moon'), 'moon');
  assert.equal(E.findWord(['moon'], 'Moonlight Sonata'), null, 'a whole word, not a part of one');
  assert.equal(E.findWord(['étoile'], 'Les Etoiles'), null, 'no plural is assumed');
  assert.equal(E.findWord(['étoile'], 'Une ETOILE filante'), 'étoile');
  assert.equal(E.findWord(['luna'], 'Claro de luna'), 'luna');
  assert.equal(E.findWord(['луна'], 'Луна и солнце'), 'луна');
  assert.equal(E.findWord(['雨'], '雨の日'), '雨');
  assert.equal(E.findWord(['꿈'], '꿈 속에서'), '꿈');
  assert.equal(E.findWord(['सपना'], 'मेरा सपना'), 'सपना');
  assert.equal(E.findWord(['rain'], ''), null);
});

test('a clock window, a date, a weekday, and one that wraps past midnight belonging to the day it starts', () => {
  const owl = { from: '03:00', to: '04:00' };
  assert.equal(E.clock(owl, at(2026, 11, 5, 3, 30)).on, true);
  assert.equal(E.clock(owl, at(2026, 11, 5, 4, 0)).on, false);
  const eve = { date: '12-31', from: '23:59:50', to: '24:00', countdown: true };
  assert.deepEqual(E.clock(eve, at(2026, 12, 31, 23, 59, 52)), { on: true, count: 8 });
  assert.equal(E.clock(eve, at(2026, 12, 30, 23, 59, 52)).on, false);
  const late = { from: '22:00', to: '02:00', weekday: 'fri' };
  assert.equal(E.clock(late, at(2026, 11, 7, 1, 0)).on, true, 'Saturday 01:00 is Friday night');
  assert.equal(E.clock(late, at(2026, 11, 8, 1, 0)).on, false);
  assert.equal(E.clock({ date: '10-31' }, at(2026, 10, 31, 12)).on, true);
});

test('the next clock change is the next edge, or the next second while a countdown runs', () => {
  assert.equal(E.nextClockChange([{ from: '03:00', to: '04:00' }], at(2026, 11, 5, 2, 59, 0)), 60000);
  assert.equal(E.nextClockChange([{ from: '23:59:50', to: '24:00', countdown: true }], at(2026, 12, 31, 23, 59, 55)), 1000);
});

test('once: a night runs 18:00 to 06:00, a song is its queue entry, always is never kept', () => {
  assert.equal(E.period('night', at(2026, 11, 6, 3), 'q1', 's'), E.period('night', at(2026, 11, 5, 22), 'q2', 's'));
  assert.notEqual(E.period('night', at(2026, 11, 6, 7), 'q', 's'), E.period('night', at(2026, 11, 6, 3), 'q', 's'));
  assert.equal(E.period('song', at(2026, 1, 1), 'q1', 's'), 'song:q1');
  assert.equal(E.period('always', at(2026, 1, 1), 'q1', 's'), null);
  assert.equal(E.period('week', at(2026, 11, 8), 'q', 's'), E.period('week', at(2026, 11, 2), 'q', 's'), 'Monday to Sunday');
});

test('a milestone fires as the count reaches it, once', () => {
  assert.equal(E.milestone([100, 500], 99, 100), 100);
  assert.equal(E.milestone([100, 500], 100, 101), null);
  assert.equal(E.milestone(1000, 998, 1003), 1000);
});

test('an egg holds when all its conditions do, and carries its count and word', () => {
  const owl = { name: 'night-owl', when: [{ clock: { from: '03:00', to: '04:00' } }, { player: { playing: true } }] };
  assert.equal(E.holds(owl, { now: at(2026, 11, 5, 3, 10), playing: true }).on, true);
  assert.equal(E.holds(owl, { now: at(2026, 11, 5, 3, 10), playing: false }).on, false);
  const moon = { name: 'moon', when: [{ words: { in: ['title', 'artist'], any: ['moon', 'luna'] } }] };
  assert.deepEqual(E.holds(moon, { track: { title: 'Blue', artist: 'Luna Park' } }), { on: true, count: null, word: 'luna' });
  const credits = { name: 'credits', when: [{ gesture: { on: 'brand', taps: 7, within: 3 } }] };
  assert.equal(E.holds(credits, { moment: { kind: 'gesture', on: 'brand', taps: 6 } }).on, false);
  assert.equal(E.holds(credits, { moment: { kind: 'gesture', on: 'brand', taps: 7 } }).on, true);
  const likes = { name: 'm', when: [{ milestone: { of: 'likes', at: [100] } }] };
  assert.equal(E.holds(likes, { moment: { kind: 'milestone', on: 'likes', before: 99, after: 100 } }).count, 100);
  const boot = { name: 'rare', when: [{ event: { on: 'boot' } }, { chance: { one_in: 40 } }] };
  assert.equal(E.holds(boot, { moment: { kind: 'event', on: 'boot' }, roll: 0.01 }).on, true);
  assert.equal(E.holds(boot, { moment: { kind: 'event', on: 'boot' }, roll: 0.5 }).on, false);
  const wait = { name: 'w', when: [{ player: { status: ['buffering'], playing: false, held: 20 } }] };
  assert.equal(E.held(wait), 20);
  assert.equal(E.moment(wait), false);
  assert.equal(E.moment(credits), true);
  assert.equal(E.line('{{count}} liked songs', 100), '100 liked songs');
});
