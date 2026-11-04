// The player's rules (web/player-rules.js) against tests/player-rules.json, the cases the desktop's native Classic
// player runs too (app/src/desktop/classic_native/rules.rs): a rule changed here fails until both match the cases.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';

const require = createRequire(import.meta.url);
const R = require('../../web/player-rules.js');
const rules = JSON.parse(readFileSync(new URL('../player-rules.json', import.meta.url), 'utf8'));
const cases = (section) => rules[section].cases || rules[section];

/* numbers to 1e-9, everything else exactly */
function near(actual, expected, what) {
  if (typeof expected === 'number' && typeof actual === 'number') assert.ok(Math.abs(actual - expected) < 1e-9, `${what}: ${actual} vs ${expected}`);
  else if (Array.isArray(expected)) { assert.equal(actual.length, expected.length, what); expected.forEach((e, i) => near(actual[i], e, `${what}[${i}]`)); }
  else if (expected && typeof expected === 'object') {
    assert.deepEqual(Object.keys(actual).sort(), Object.keys(expected).sort(), what);
    for (const k of Object.keys(expected)) near(actual[k], expected[k], `${what}.${k}`);
  } else assert.equal(actual, expected, what);
}

const run = {
  nextIndex: (c) => R.nextIndex({ index: c.index, length: c.length, repeat: c.repeat }, c.auto),
  previous: (c) => R.previous(c.index, c.pos),
  cycleRepeat: (c) => R.cycleRepeat(c.mode),
  playStart: (c) => R.playStart(c.trackId, c.duration, c.recommended),
  playEnd: (c) => R.playEnd(c.heard, c.how),
  outputVolume: (c) => R.outputVolume(c.volume, c.muted),
  replayGainDb: (c) => R.replayGainDb(c.replayGain, c.mode),
  gain: (c) => R.gain(c.db),
  loudness: (c) => R.loudness(c.targetDb, c.rms, c.readings),
  crossfadeS: (c) => R.crossfadeS(c.repeat, c.crossfade),
  preloadLead: (c) => R.preloadLead(c.crossfade, c.soulseek, c.received),
  preloadNow: (c) => R.preloadNow(c.lead, c.duration, c.pos),
  gapless: (c) => R.gapless(c.duration, c.pos),
  endedEarly: (c) => R.endedEarly(c.soulseek, c.complete, c.duration, c.pos),
  fade: (c) => R.fade(c.k),
  crossfadeLengthS: (c) => R.crossfadeLengthS(c.left),
  upcoming: (c) => R.upcoming({ index: c.index, ids: c.ids, shuffle: c.shuffle, repeat: c.repeat }, (id) => c.saved.includes(id), c.n),
  radio: (c) => R.radio({ index: c.index, length: c.length, repeat: c.repeat, radio: c.radio }, c.radioWhenQueueEnds),
  seekLimit: (c) => R.seekLimit(c.soulseek, c.buffered, c.received),
  seekTo: (c) => R.seekTo(c.fraction, c.limit),
  switchNow: (c) => R.switchNow(c),
  quiet: (c) => R.quiet(c.levels),
  levelMatch: (c) => R.levelMatch(c.song, c.copy),
};

test('every section of the cases is a rule here, and every rule here has cases', () => {
  const sections = Object.keys(rules).filter((k) => k !== 'note' && k !== 'constants' && k !== 'listening');
  assert.deepEqual(sections.sort(), Object.keys(run).sort());
});

test('the timings are the shared ones', () => {
  for (const [name, value] of Object.entries(rules.constants)) assert.equal(R[name], value, name);
});

for (const [section, apply] of Object.entries(run)) {
  test(section, () => {
    cases(section).forEach((c, i) => near(apply(c), c.expect, `${section} #${i}`));
  });
}

/* the order app.js's time handler applies them (trackListening, then beginPlay, whose play starts with no last
   position): what the cases' `starts` and `heard` describe */
test('listening', () => {
  for (const [i, c] of rules.listening.cases.entries()) {
    let play = null, starts = -1;
    c.reports.forEach(([pos, playing, waiting], k) => {
      if (play) { play.heard += R.heard(play.last == null ? 0 : pos - play.last, playing); play.last = pos; }
      if (!play && playing && !waiting) { play = { heard: 0, last: null }; starts = k; }
    });
    assert.equal(starts, c.starts, `listening #${i} starts`);
    near(play ? play.heard : 0, c.heard, `listening #${i} heard`);
  }
});
