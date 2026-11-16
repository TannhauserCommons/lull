// The one player view (ADR-0041, web/player-view.js): every surface (the bar, Now Playing, Home's lead, the live model,
// Connect, the mini player windows) shows what PlayerView.derive() makes of the player's state, so "what is the player
// doing" has one answer instead of the seven rules that disagreed (P.heard, the mini's runs, the wave's moving, the play
// keys' playingNow, the bar's data-playing, Connect's heartbeat). These pin derive's activity table, row by row and in
// its first-match order, and the clock every surface places the song with. Run: node --test tests/web
//
// derive(input): the input is plain values the page's player already has (app.js), named as it names them:
//   track          P.current(): the catalogue's song ({ id, title, artist, album, format, duration, ... }) or null
//   status         ps.status: idle | finding | choose | queued | buffering | streaming | stalled | failed | library |
//                  ended | jellyfin-error
//   intent         ps.playing: what the listener asked for (true: play)
//   sound          what the output last reported ({ playing, waiting }, as its `time` events say), or null before it
//                  has reported anything for this song (the output hasn't started)
//   words          P.stateWords()
//   copy           P.playingCopy() (LullRules.playingCopy: { where, name, library, peer, saved, received, format }) or null
//   pos            ps.pos, seconds
//   duration       P.duration(), seconds
//   durationKnown  ps.durationKnown
//   received       ps.received, 0 to 1
//   front, parts   P.transferBar(): the solid part from the start, and the parts drawn as arrived ([[from, to], ...])
//   seekWait       ps.seekWait: { fraction } or null
//   liked, saved   App.isLiked(id), App.isSaved(id)
//   shuffle, repeat, volume, muted   ps.shuffle, ps.repeat, ps.volume, ps.muted
//   crossfading    ps.crossfade
//   now            Date.now(): the time, in ms, the clock's pos is true at
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const V = require('../../web/player-view.js');

const NOW = 1_800_000_000_000;
const song = { id: 't-1', title: 'Velvet Hour', artist: 'Quiet Canal', album: 'Low Tide', format: 'FLAC 16/44.1', lossless: true, duration: 200, albumId: 'al-1', artistId: 'ar-1' };
const copy = { where: 'soulseek', name: 'quietcanal', library: 'folder', peer: { name: 'quietcanal', format: 'MP3 320' }, saved: false, received: 0.4, format: 'MP3 320' };
/* a song streaming from a peer and heard: the listener asked to play, the output plays and isn't waiting */
const input = (over) => ({
  track: song, status: 'streaming', intent: true, sound: { playing: true, waiting: false }, words: 'Streaming from quietcanal',
  copy, pos: 20, duration: 200, durationKnown: true, received: 0.4, front: 0.4, parts: [[0, 0.4]], seekWait: null,
  liked: false, saved: false, shuffle: false, repeat: 'off', volume: 0.7, muted: false, crossfading: false, now: NOW,
  ...over,
});
const activity = (over) => V.derive(input(over)).activity;

test('the view carries the wire version, so a mini player and a main that differ notice it', () => {
  assert.ok(Number.isInteger(V.VERSION) && V.VERSION >= 1, 'VERSION is a whole number the view\'s shape raises');
  assert.equal(V.derive(input()).v, V.VERSION);
  assert.equal(V.derive(input({ track: null, status: 'idle', intent: false, sound: null, copy: null })).v, V.VERSION);
});

test('with no song, or status idle, the player is idle, whatever else it says', () => {
  assert.equal(activity({ track: null }), 'idle');
  assert.equal(activity({ track: null, status: 'failed' }), 'idle', 'no song comes before a failure');
  assert.equal(activity({ status: 'idle' }), 'idle');
  assert.equal(activity({ status: 'idle', intent: false, sound: null }), 'idle');
});

test('a failed song is an error, and one the listener left playing is still an error, not playing or paused', () => {
  for (const status of ['failed', 'jellyfin-error']) {
    assert.equal(activity({ status }), 'error', status);
    assert.equal(activity({ status, intent: false }), 'error', `${status}, paused: error comes before paused`);
    assert.equal(activity({ status, sound: { playing: true, waiting: false } }), 'error', `${status} with the output still going`);
  }
});

test('an ended song is ended, before paused', () => {
  assert.equal(activity({ status: 'ended', intent: false }), 'ended');
  assert.equal(activity({ status: 'ended', intent: true }), 'ended');
});

test('a song waiting for the listener to pick a copy is choose, before paused', () => {
  assert.equal(activity({ status: 'choose', intent: true, sound: null }), 'choose');
  assert.equal(activity({ status: 'choose', intent: false, sound: null }), 'choose');
});

test('the listener\'s pause is paused, even while the song streams, plays from the library or still loads', () => {
  for (const status of ['streaming', 'library', 'finding', 'queued', 'buffering', 'stalled']) {
    assert.equal(activity({ status, intent: false }), 'paused', `${status}, the output still reporting sound`);
    assert.equal(activity({ status, intent: false, sound: { playing: false, waiting: false } }), 'paused', status);
    assert.equal(activity({ status, intent: false, sound: null }), 'paused', `${status}, before the output started`);
  }
});

test('finding, queued, buffering and stalled are loading, even with sound coming out', () => {
  for (const status of ['finding', 'queued', 'buffering', 'stalled']) {
    assert.equal(activity({ status, sound: null }), 'loading', status);
    assert.equal(activity({ status, sound: { playing: true, waiting: false } }), 'loading', `${status}: only streaming or library plays`);
    assert.equal(activity({ status, sound: { playing: true, waiting: true } }), 'loading', status);
  }
});

test('streaming or library while the output waits for data, or hasn\'t started, is loading', () => {
  for (const status of ['streaming', 'library']) {
    assert.equal(activity({ status, sound: { playing: true, waiting: true } }), 'loading', `${status}, the output waiting`);
    assert.equal(activity({ status, sound: null }), 'loading', `${status}, the output not started`);
    assert.equal(activity({ status, sound: { playing: false, waiting: false } }), 'loading', `${status}, the output not playing yet`);
  }
});

test('streaming or library with the output playing and not waiting is playing', () => {
  assert.equal(activity({ status: 'streaming' }), 'playing');
  assert.equal(activity({ status: 'library' }), 'playing');
});

test('the clock moves only while the player is playing', () => {
  const cases = {
    playing: {}, paused: { intent: false }, loading: { status: 'buffering' }, idle: { track: null }, error: { status: 'failed' },
    ended: { status: 'ended', intent: false }, choose: { status: 'choose' }, 'loading (output waits)': { sound: { playing: true, waiting: true } },
  };
  for (const [name, over] of Object.entries(cases)) {
    const v = V.derive(input(over));
    assert.equal(v.clock.rate, v.activity === 'playing' ? 1 : 0, name);
  }
  assert.equal(V.derive(input()).clock.rate, 1);
});

test('the view says where the song is and when that was true, with its length', () => {
  const v = V.derive(input({ pos: 42.5, duration: 180, durationKnown: false }));
  assert.deepEqual(v.clock, { pos: 42.5, at: NOW, rate: 1, duration: 180, known: false });
});

test('the view carries the song as the surfaces name it, the copy that plays, the transfer and the listener\'s settings', () => {
  const v = V.derive(input({ liked: true, saved: true, shuffle: true, repeat: 'one', volume: 0.3, muted: true, crossfading: true,
    seekWait: { fraction: 0.6 }, received: 0.55, front: 0.5, parts: [[0, 0.5], [0.7, 0.8]] }));
  assert.equal(v.status, 'streaming');
  assert.equal(v.words, 'Streaming from quietcanal');
  assert.deepEqual(v.track, { id: 't-1', title: 'Velvet Hour', artist: 'Quiet Canal', album: 'Low Tide' });
  assert.equal(v.copy.format, 'MP3 320', 'the copy that plays, not the catalogue\'s FLAC');
  assert.equal(v.copy.where, 'soulseek');
  assert.equal(v.copy.received, 0.4);
  assert.equal(v.transfer.received, 0.55);
  assert.equal(v.transfer.front, 0.5);
  assert.deepEqual(v.transfer.parts, [[0, 0.5], [0.7, 0.8]]);
  assert.equal(v.seekWait, 0.6, 'the fraction the pending seek lands at');
  assert.deepEqual([v.liked, v.saved, v.shuffle, v.repeat, v.volume, v.muted, v.crossfading], [true, true, true, 'one', 0.3, true, true]);
  const none = V.derive(input({ track: null, status: 'idle', copy: null, seekWait: null }));
  assert.equal(none.track, null);
  assert.equal(none.copy, null);
  assert.equal(none.seekWait, null);
});

test('posAt moves a playing song on by the time since its clock, and holds a paused one', () => {
  const playing = V.derive(input({ pos: 20 }));
  assert.equal(V.posAt(playing, NOW), 20);
  assert.equal(V.posAt(playing, NOW + 3000), 23);
  assert.equal(V.posAt(playing, NOW + 250), 20.25);
  const paused = V.derive(input({ pos: 20, intent: false }));
  assert.equal(V.posAt(paused, NOW + 3000), 20);
  const loading = V.derive(input({ pos: 20, status: 'buffering' }));
  assert.equal(V.posAt(loading, NOW + 3000), 20, 'a loading song stays where it waits');
});

test('posAt never places the song past its end', () => {
  const v = V.derive(input({ pos: 199, duration: 200 }));
  assert.equal(V.posAt(v, NOW + 5000), 200);
  assert.equal(V.posAt(v, NOW + 600_000), 200);
});

test('a view\'s key ignores where its clock says the song is, and changes with anything else', () => {
  const base = V.derive(input());
  const moved = { ...base, clock: { ...base.clock, pos: 99, at: NOW + 79_000 } };
  assert.deepEqual(V.key(moved), V.key(base), 'the position and its time never make a view new');
  const other = {
    v: base.v + 1, activity: 'paused', status: 'library', words: 'Playing from your library',
    track: { ...base.track, title: 'Another Song' }, copy: { ...base.copy, format: 'FLAC 24/96' },
    clock: { ...base.clock, rate: 0 }, transfer: { ...base.transfer, received: 0.9 }, seekWait: 0.3,
    liked: true, saved: true, shuffle: true, repeat: 'all', volume: 0.2, muted: true, crossfading: true,
  };
  for (const [field, value] of Object.entries(other)) {
    assert.notDeepEqual(V.key({ ...base, [field]: value }), V.key(base), field);
  }
  assert.notDeepEqual(V.key({ ...base, clock: { ...base.clock, duration: 201 } }), V.key(base), 'clock.duration');
  assert.notDeepEqual(V.key({ ...base, clock: { ...base.clock, known: false } }), V.key(base), 'clock.known');
});

test('a clock a second or more off from the last view\'s has strayed; one running where it said has not', () => {
  const prev = V.derive(input({ pos: 20 }));
  const at = (pos, now, over) => V.derive(input({ pos, now, ...over }));
  const later = NOW + 3000;
  assert.equal(V.strayed(prev, at(23, later), later), false, 'right where the clock said');
  assert.equal(V.strayed(prev, at(23.2, later), later), false, '0.2 s off');
  assert.equal(V.strayed(prev, at(22.8, later), later), false, '0.2 s behind');
  assert.equal(V.strayed(prev, at(24, later), later), true, 'a second ahead (a seek, a drifting clock)');
  assert.equal(V.strayed(prev, at(21.5, later), later), true, 'a second and a half behind (a stall)');
  assert.equal(V.strayed(prev, at(80, later), later), true, 'a seek');
  const paused = V.derive(input({ pos: 20, intent: false }));
  assert.equal(V.strayed(paused, at(20, later, { intent: false }), later), false, 'a paused song where it stopped');
  assert.equal(V.strayed(paused, at(60, later, { intent: false }), later), true, 'a paused song sought elsewhere');
});
