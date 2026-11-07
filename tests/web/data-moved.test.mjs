// web/data.js: a merge that moves a song to another album, or gives an album a cover, tells M.onMoved, so the player bar and
// the queue panel (app.js) draw the song again. Live, a song played from Home had no cover in either until a lookup after the
// play moved it to the catalog's album, and the queue panel was not redrawn when it did.
// Run: node --test tests/web
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

function load() {
  const window = {};
  runInNewContext(readFileSync(new URL('../../web/data.js', import.meta.url), 'utf8'), { window, console });
  const M = window.LullData;
  const moved = [];
  M.onMoved = (ids) => moved.push(Object.keys(ids).sort());
  return { M, moved };
}

const song = (albumId) => ({ id: 't-1', title: 'Paper Moon Static', artist: 'Velvet Orchard', albumId });

test('a song moved to another album is announced, once the merge is whole', () => {
  const { M, moved } = load();
  M.absorb({ tracks: { 't-1': song('al-old') } });
  assert.deepEqual(moved, [], 'a song seen first is not moved');
  M.absorb({ tracks: { 't-1': song('al-new') }, albums: { 'al-new': { id: 'al-new', title: 'Satellite Porch', art: 'api/catalog/art/al-new' } } });
  assert.equal(moved.length, 1);
  assert.deepEqual(moved[0], ['al-new', 't-1']);
  assert.equal(M.track('t-1').albumId, 'al-new');
  assert.equal(M.album(M.track('t-1').albumId).art, 'api/catalog/art/al-new', 'its album is there when it is announced');
});

test('an album that gains a cover is announced; the same cover, or a song that says nothing of its album, is not', () => {
  const { M, moved } = load();
  M.absorb({ albums: { 'al-1': { id: 'al-1', title: 'Satellite Porch' } }, tracks: { 't-1': song('al-1') } });
  moved.length = 0;
  M.absorb({ albums: { 'al-1': { id: 'al-1', art: 'api/catalog/art/al-1' } } });
  assert.deepEqual(moved, [['al-1']]);
  M.absorb({ albums: { 'al-1': { id: 'al-1', art: 'api/catalog/art/al-1' } }, tracks: { 't-1': song('al-1') } });
  M.absorb({ tracks: { 't-1': { id: 't-1', duration: 200 } } });
  assert.equal(moved.length, 1, 'nothing moved');
});
