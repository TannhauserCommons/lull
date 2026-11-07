// The copy that plays (LullRules.playingCopy, web/player-rules.js): the player bar, Now Playing and source details all
// say where a song comes from by it, so they can't disagree. A song streaming from Soulseek before any peer has
// answered is a Soulseek copy, not the library's (it was read as "your library via Jellyfin", with an empty chip).
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const R = require('../../web/player-rules.js');

const song = { id: 't-1', format: '' };
const flac = { id: 't-1', format: 'FLAC 16/44.1' };
const copy = (over) => R.playingCopy({ track: song, e: { soulseek: true, url: 'api/stream/t-1' }, status: 'streaming', peer: null, saved: false, received: 0, library: 'folder', ...over });

test('an unsaved song streaming before any peer has answered is a Soulseek copy, with no format to show', () => {
  const c = copy({ received: 0.31 });
  assert.equal(c.where, 'soulseek');
  assert.equal(c.name, null);
  assert.equal(c.format, null);
  assert.equal(c.peer, null);
  assert.equal(c.received, 0.31);
  assert.equal(c.saved, false);
});

test('a Soulseek copy takes its name and format from the peer once it answers', () => {
  const c = copy({ peer: { name: 'quietcanal', format: 'MP3 320' }, received: 0.5 });
  assert.equal(c.where, 'soulseek');
  assert.equal(c.name, 'quietcanal');
  assert.equal(c.format, 'MP3 320');
  assert.equal(c.peer.name, 'quietcanal');
});

test('a peer that sent no format falls back to the song\'s own', () => {
  assert.equal(copy({ track: flac, peer: { name: 'quietcanal' } }).format, 'FLAC 16/44.1');
});

test('a song saved while it streams is still the peer\'s copy, and says it is saved', () => {
  const c = copy({ saved: true, peer: { name: 'quietcanal', format: 'FLAC 16/44.1' }, received: 0.6 });
  assert.equal(c.where, 'soulseek');
  assert.equal(c.saved, true);
  assert.equal(c.received, 0.6);
});

test('the library is named for what it is: the music folder, Jellyfin, or just the library until the hub says', () => {
  const lib = { e: { soulseek: false, url: 'api/library/stream/t-1', complete: true }, status: 'library', saved: true, received: 1, track: flac };
  assert.equal(R.playingCopy({ ...lib, library: 'folder' }).name, 'your music folder');
  assert.equal(R.playingCopy({ ...lib, library: 'jellyfin' }).name, 'Jellyfin');
  assert.equal(R.playingCopy({ ...lib, library: null }).name, 'your library');
  const c = R.playingCopy({ ...lib, library: 'folder' });
  assert.equal(c.where, 'library');
  assert.equal(c.format, 'FLAC 16/44.1');
  assert.equal(c.peer, null);
});

test('a library copy ignores a peer left over from an earlier source, and shows no chip for a format nobody knows', () => {
  const c = R.playingCopy({ track: song, e: { soulseek: false, url: 'api/library/stream/t-1' }, status: 'library', peer: { name: 'old', format: 'MP3 128' }, saved: true, received: 1, library: 'jellyfin' });
  assert.equal(c.where, 'library');
  assert.equal(c.peer, null);
  assert.equal(c.format, null);
});

test('a download on this device is the device\'s copy', () => {
  const c = R.playingCopy({ track: flac, e: { local: true, soulseek: false, url: 'blob:x' }, status: 'library', saved: true, received: 1, library: 'jellyfin' });
  assert.equal(c.where, 'device');
  assert.equal(c.name, 'this device');
  assert.equal(c.format, 'FLAC 16/44.1');
});

test('YouTube is its own place, and its copy\'s format is the chip', () => {
  const c = copy({ peer: { name: 'YouTube', format: 'YouTube · Opus 160' } });
  assert.equal(c.where, 'youtube');
  assert.equal(c.name, 'YouTube');
  assert.equal(c.format, 'YouTube · Opus 160');
});

test('before the source is known, a saved song is the library\'s and an unsaved one is Soulseek\'s', () => {
  assert.equal(R.playingCopy({ track: flac, e: { trackId: 't-1' }, status: 'library', saved: true, library: 'folder' }).where, 'library');
  assert.equal(R.playingCopy({ track: song, e: { trackId: 't-1' }, status: 'finding', saved: false, library: 'folder' }).where, 'soulseek');
  assert.equal(R.playingCopy({ track: song, e: { trackId: 't-1' }, status: 'streaming', saved: false, library: 'folder' }).where, 'soulseek');
});

test('a library item Jellyfin can\'t serve is still the library\'s copy', () => {
  assert.equal(R.playingCopy({ track: flac, e: { trackId: 't-1' }, status: 'jellyfin-error', saved: true, library: 'jellyfin' }).where, 'library');
});
