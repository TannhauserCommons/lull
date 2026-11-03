// The fixture hub: serves this checkout's web/ from disk and answers /api from library.mjs, so the UI
// gates (perf, screens, a11y) run anywhere with Node alone: no image, no Soulseek, no outside service,
// the same answers on every run. It is a stand-in for the hub's contract (docs/context/api.md), not a
// second hub: anything the UI asks that is not modelled here answers 404 `not_found`, which the specs
// count as a failure, so a new endpoint shows up as a gap to fill rather than as a silent blank.
//
//   node tests/ui/fixture-hub/server.mjs [port]      (default 8790; LULL_WEB=<dir> serves another web/)
//
// POST /__fixture/emit { topic, data } sends one server event to every open /api/events stream.
// POST /__fixture/art-pending { ids } makes those covers answer 404 "not yet" (no-store), as the hub does while it
// fetches one; POST /__fixture/art-ready { ids } serves them again and sends `art.ready { keys }`.
// GET /__fixture/art-requests: cover requests that reached the hub since the last call, by URL (and resets them).

import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { deflateSync } from 'node:zlib';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as L from './library.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const WEB = process.env.LULL_WEB || join(here, '..', '..', '..', 'web');
const PORT = +(process.argv[2] || process.env.FIXTURE_PORT || 8790);

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2', '.webmanifest': 'application/manifest+json',
  '.ico': 'image/x-icon', '.txt': 'text/plain', '.jpg': 'image/jpeg', '.wsz': 'application/zip',
};

// ---------- state a session can change (likes, follows, playlists, settings) ----------
const state = {
  likes: [...L.likes],
  follows: { artists: L.artistIds.slice(0, 6), tags: L.tags.slice(0, 3) },
  playlists: structuredClone(L.playlists),
  settings: {
    playback: { gapless: true, crossfade: 0, normalisation: 'off', loudnessTarget: -14, volumeStep: 5, radioWhenQueueEnds: false, prefetchCount: 2 },
    sources: {
      maxPeerQueue: 20, stallTimeout: 15,
      stream: { below: 'playBest', maxKbps: 0, minimum: { formats: [], kbps: 192, losslessOnly: false }, pick: 'fastest' },
      save: { autoUpgrade: true, below: 'saveBestUpgrade', minimum: { formats: [], kbps: 320, losslessOnly: false }, pick: 'highest', upgradeEveryH: 24 },
      youtube: { mode: 'off', format: 'opus', match: 'close', waitS: 20, saves: false, autoUpdate: true },
      preferredFormats: { source: 'config', value: ['flac', 'mp3', 'opus'] },
    },
    search: { showFiles: false },
    library: { duplicatePolicy: 'ask', musicFolder: { source: 'config', value: '/music' } },
    sharing: { uploadSlots: 3, uploadLimitKBs: 500, mode: { source: 'config', value: 'library' } },
    discovery: { adventurousness: 0.4, on: { listenbrainz: true, musicbrainz: true, lastfm: true } },
    cache: { capGB: { source: 'config', value: 10 }, usedGB: 1.2 },
    app: { version: '0.0.0-fixture', server: { source: 'config', value: 'http://127.0.0.1' } },
  },
  plays: 0,
};
const clients = new Set();

// ---------- helpers ----------
function entities({ tracks = [], albums = [], artists = [] } = {}) {
  const e = { tracks: {}, albums: {}, artists: {} };
  for (const id of tracks) if (L.tracks[id]) e.tracks[id] = L.tracks[id];
  for (const id of albums) if (L.albums[id]) e.albums[id] = L.albums[id];
  for (const id of artists) if (L.artists[id]) e.artists[id] = L.artists[id];
  return e;
}
const albumsOf = (trackIds) => [...new Set(trackIds.map((t) => L.tracks[t]?.albumId).filter(Boolean))];
const page = (items, q) => {
  const offset = +(q.get('offset') || 0), limit = +(q.get('limit') || 200);
  return items.slice(offset, offset + limit);
};
const err = (status, error, message) => ({ status, body: { error, message } });
const words = (q) => (q || '').toLowerCase().trim().replace(/\s+/g, ' ');
const has = (text, q) => text.toLowerCase().includes(q);

function playlistBody(p) {
  return { ...p, entities: entities({ tracks: p.items.map((i) => i.trackId), albums: albumsOf(p.items.map((i) => i.trackId)) }) };
}

const SERVICES = {
  soulseek: { state: 'ok', detail: 'Connected as fixture' },
  library: { state: 'ok', detail: `Music folder: ${L.savedIds.length} songs`, extra: { mode: 'folder', songs: L.savedIds.length } },
  lastfm: { state: 'ok', detail: 'Connected as fixture', extra: { queued: 0 } },
  cache: { state: 'ok', detail: '1.2 GB of 10.0 GB used, oldest audio evicted first', extra: { usedBytes: 1.2e9, capBytes: 1e10 } },
  catalog: { state: 'ok', detail: 'catalog: the fixture library' },
  youtube: { state: 'off', detail: 'YouTube is off' },
};

// ---------- routes: [method, regex, handler(match, query, body) -> { status?, body } | body] ----------
const routes = [
  ['GET', /^\/health$/, () => ({ version: '0.0.0-fixture', services: SERVICES })],
  ['GET', /^\/api\/ping$/, () => ({ status: 204 })],
  ['GET', /^\/api\/setup$/, () => ({ claimed: true, soulseekManaged: false, steps: { library: true, soulseek: true, lastfm: true } })],
  ['GET', /^\/api\/devices$/, () => ({
    me: 'd-fixture',
    items: [
      { id: 'd-fixture', name: 'This browser', kind: 'browser', createdAt: L.NOW - 86400 * 30, lastSeen: L.NOW, private: false, online: true },
      { id: 'd-phone', name: 'Phone', kind: 'app', createdAt: L.NOW - 86400 * 20, lastSeen: L.NOW - 7200, private: false, online: false },
    ],
  })],
  ['PATCH', /^\/api\/devices\/me$/, () => ({ status: 204 })],
  ['POST', /^\/api\/devices\/me\/state$/, () => ({ status: 204 })],
  ['GET', /^\/api\/config$/, () => ({ items: [
    { key: 'LULL_PUBLIC_URL', value: null, set: false, source: 'default', secret: false, restart: true },
    { key: 'LULL_SLSKD_URL', value: 'http://slskd:5030', set: true, source: 'env', secret: false, restart: true },
    { key: 'LULL_SLSKD_API_KEY', value: null, set: true, source: 'env', secret: true, restart: true },
  ] })],
  ['GET', /^\/api\/settings$/, () => state.settings],
  ['PATCH', /^\/api\/settings$/, (m, q, body) => { deepMerge(state.settings, body || {}); return state.settings; }],
  ['GET', /^\/api\/themes$/, () => ({ items: [] })],
  ['GET', /^\/api\/lastfm$/, () => ({ configured: true, connected: true, user: 'fixture', scrobbling: true, useHistory: true, queued: 0 })],
  ['GET', /^\/api\/youtube$/, () => ({ state: 'off', detail: 'YouTube is off', mode: 'off', ytdlp: { version: null, updated: false, pinnedVersion: null, latest: null, checkedAt: null, error: null }, backoffUntil: null })],
  ['GET', /^\/api\/library\/source$/, () => ({ mode: 'folder', musicDir: '/music', jellyfin: null, files: L.savedIds.length, lastScan: L.NOW - 3600, switching: false })],
  ['GET', /^\/api\/jobs$/, () => ({ items: [] })],
  ['GET', /^\/api\/transfers$/, () => ({ items: [], total: 0 })],
  ['GET', /^\/api\/import\/links$/, () => ({ links: [] })],
  ['GET', /^\/api\/bookmarks$/, () => ({ items: [] })],
  ['GET', /^\/api\/soulseek\/diagnostics$/, () => ({ slskd: { version: 'fixture', server: { state: 'Connected, LoggedIn' }, distributedNetwork: null, connectionWatchdog: null, pendingReconnect: false }, searches: [], log: [] })],

  // collection
  ['GET', /^\/api\/likes$/, (m, q) => ({ items: page(state.likes, q), total: state.likes.length, entities: entities({ tracks: page(state.likes, q), albums: albumsOf(page(state.likes, q)) }) })],
  ['PUT', /^\/api\/likes\/(.+)$/, (m) => { if (!state.likes.includes(m[1])) state.likes.unshift(m[1]); emit('like', { trackId: m[1], liked: true }); return { status: 204 }; }],
  ['DELETE', /^\/api\/likes\/(.+)$/, (m) => { state.likes = state.likes.filter((t) => t !== m[1]); emit('like', { trackId: m[1], liked: false }); return { status: 204 }; }],
  ['GET', /^\/api\/follows$/, () => ({ ...state.follows, entities: entities({ artists: state.follows.artists }) })],
  ['GET', /^\/api\/playlists$/, () => ({ items: state.playlists, total: state.playlists.length, entities: entities({ tracks: state.playlists.flatMap((p) => p.items.slice(0, 4).map((i) => i.trackId)), albums: albumsOf(state.playlists.flatMap((p) => p.items.slice(0, 4).map((i) => i.trackId))) }) })],
  ['GET', /^\/api\/playlists\/(pl-[^/]+)$/, (m) => { const p = state.playlists.find((x) => x.id === m[1]); return p ? playlistBody(p) : err(404, 'not_found', 'No such playlist.'); }],
  ['PUT', /^\/api\/playlists\/(pl-[^/]+)\/tracks$/, (m, q, body) => {
    const p = state.playlists.find((x) => x.id === m[1]); if (!p) return err(404, 'not_found', 'No such playlist.');
    const added = Object.fromEntries(p.items.map((i) => [i.trackId, i.added]));
    p.items = body.map((trackId) => ({ trackId, added: added[trackId] || L.NOW })); p.updated = L.NOW;
    return playlistBody(p);
  }],
  ['GET', /^\/api\/history$/, (m, q) => {
    const all = L.history.flatMap((d) => d.items);
    return { days: q.get('before') ? [] : L.history, next: null, entities: entities({ tracks: all.map((i) => i.trackId), albums: albumsOf(all.map((i) => i.trackId)) }) };
  }],
  ['POST', /^\/api\/plays$/, () => ({ playId: String(++state.plays) })],
  ['PATCH', /^\/api\/plays\/(.+)$/, (m) => ({ playId: m[1] })],
  ['GET', /^\/api\/stats$/, () => {
    const top = L.trackIds.slice(0, 10);
    return {
      minutes: 4210, plays: 812, artists: 48, saves: 31, skipRate: 0.18, newArtists: 9,
      hitRate: { recommended: 0.42, liked: 0.12, saved: 0.08 },
      topArtists: L.artistIds.slice(0, 8).map((id, i) => [id, 90 - i * 9]), topTracks: top.map((id, i) => [id, 40 - i * 3]),
      topTags: L.tags.slice(0, 6).map((t, i) => [t, 30 - i * 4]), clock: Array.from({ length: 24 }, (_, h) => Math.round(20 + 18 * Math.sin((h - 6) / 3.8))),
      entities: entities({ tracks: top, artists: L.artistIds.slice(0, 8), albums: albumsOf(top) }),
    };
  }],
  ['GET', /^\/api\/taste$/, () => ({ artists: L.artistIds.slice(0, 12).map((id, i) => [id, 1 - i / 14]), tags: L.tags.map((t, i) => [t, 1 - i / 13]), entities: entities({ artists: L.artistIds.slice(0, 12) }) })],

  // library
  ['GET', /^\/api\/library\/songs$/, (m, q) => {
    const f = words(q.get('filter'));
    let items = L.savedIds.filter((id) => !f || has(`${L.tracks[id].title} ${L.tracks[id].artist} ${L.tracks[id].album}`, f));
    items = sortTracks(items, q.get('sort') || 'album');
    const shown = page(items, q);
    return { items: shown, total: items.length, entities: entities({ tracks: shown, albums: albumsOf(shown) }), songs: Object.fromEntries(shown.map((id) => [id, L.songs[id]])), stale: false };
  }],
  ['GET', /^\/api\/library\/albums$/, (m, q) => {
    let items = [...L.savedAlbumIds];
    const s = q.get('sort') || 'recent';
    if (s === 'title') items.sort((a, b) => L.albums[a].title.localeCompare(L.albums[b].title));
    else if (s === 'artist') items.sort((a, b) => L.albums[a].artist.localeCompare(L.albums[b].artist));
    else if (s === 'year') items.sort((a, b) => L.albums[b].year - L.albums[a].year);
    else items.reverse();
    const shown = page(items, q);
    return { items: shown, total: items.length, entities: entities({ albums: shown }), stale: false };
  }],
  ['GET', /^\/api\/library\/artists$/, (m, q) => {
    const items = [...L.artistIds].sort((a, b) => L.artists[a].name.localeCompare(L.artists[b].name));
    const shown = page(items, q);
    return { items: shown, total: items.length, entities: entities({ artists: shown }), stale: false };
  }],
  ['GET', /^\/api\/library\/genres$/, () => {
    const items = L.tags.map((name) => {
      const ids = L.savedIds.filter((id) => L.tracks[id].tags.includes(name));
      return { id: name, name, artists: new Set(ids.map((id) => L.tracks[id].artistId)).size, songs: ids.length, trackIds: ids.slice(0, 4) };
    });
    return { items, total: items.length, entities: entities({ tracks: items.flatMap((g) => g.trackIds), albums: albumsOf(items.flatMap((g) => g.trackIds)) }), stale: false };
  }],
  ['GET', /^\/api\/tracks\/([^/]+)$/, (m) => (L.tracks[m[1]] ? { track: L.tracks[m[1]], entities: entities({ tracks: [m[1]], albums: albumsOf([m[1]]) }) } : err(404, 'not_found', 'lull doesn\'t know this song.'))],
  ['GET', /^\/api\/tracks\/([^/]+)\/sources$/, () => ({ sources: [], done: true })],
  ['PUT', /^\/api\/stream\/prefetch$/, () => ({ status: 204 })],

  // discovery
  ['GET', /^\/api\/feed$/, () => {
    const picks = L.trackIds.filter((_, i) => i % 97 === 1).slice(0, 12);
    const fresh = L.albumIds.slice(-12, -2).reverse();   // the catalog's albums, none saved
    return {
      updated: L.NOW - 600, summary: 'Dreamy guitars and late synths this week.',
      newReleases: fresh,
      picks: picks.map((trackId, i) => ({ trackId, reasons: [i % 2 ? { kind: 'tag', name: L.tracks[trackId].tags[0] } : { kind: 'like', name: L.tracks[trackId].artist, ref: L.tracks[trackId].artistId }], via: 'listenbrainz' })),
      radioStarters: L.artistIds.slice(0, 6).map((id) => ({ id: 'rs-' + id, title: `${L.artists[id].name} radio`, sub: L.artists[id].tags.join(', '), seed: { kind: 'artist', id } })),
      jumpBackIn: [...L.albumIds.slice(0, 4).map((id) => ({ kind: 'album', id })), ...L.artistIds.slice(6, 10).map((id) => ({ kind: 'artist', id }))],
      followedTags: state.follows.tags, coldStartTags: L.tags.slice(0, 8),
      entities: entities({ tracks: picks, albums: [...fresh, ...L.albumIds.slice(0, 4), ...albumsOf(picks)], artists: L.artistIds.slice(0, 10) }),
    };
  }],
  ['GET', /^\/api\/search$/, (m, q) => {
    const w = words(q.get('q'));
    if (!w) return err(400, 'bad_request', 'Search for some words.');
    const tracks = L.trackIds.filter((id) => has(`${L.tracks[id].title} ${L.tracks[id].artist}`, w)).slice(0, 50);
    const albums = L.albumIds.filter((id) => has(`${L.albums[id].title} ${L.albums[id].artist}`, w)).slice(0, 20);
    const artists = L.artistIds.filter((id) => has(L.artists[id].name, w)).slice(0, 20);
    const top = artists[0] ? { kind: 'artist', id: artists[0] } : albums[0] ? { kind: 'album', id: albums[0] } : tracks[0] ? { kind: 'track', id: tracks[0] } : null;
    return { query: w, top, artists, albums, tracks, playlists: [], pending: [], entities: entities({ tracks, albums: [...albums, ...albumsOf(tracks)], artists }) };
  }],
  ['GET', /^\/api\/soulseek\/search$/, (m, q) => {
    const searchId = 'srch-' + words(q.get('q')).replace(/\W+/g, '-');
    setTimeout(() => emit('search.results', { searchId, query: words(q.get('q')), files: 0, peers: 0, done: true, results: [], entities: {}, onlySoulseek: [] }), 50);
    return { searchId, soulseek: null, entities: {} };
  }],
  ['GET', /^\/api\/albums\/([^/]+)$/, (m) => {
    const a = L.albums[m[1]]; if (!a) return err(404, 'not_found', 'lull doesn\'t know this album.');
    return { album: a, entities: entities({ tracks: a.trackIds, albums: [a.id, ...L.albumIds.filter((x) => L.albums[x].artistId === a.artistId)], artists: [a.artistId] }) };
  }],
  ['GET', /^\/api\/artists\/([^/]+)$/, (m) => {
    const ar = L.artists[m[1]]; if (!ar) return err(404, 'not_found', 'lull doesn\'t know this artist.');
    const albums = L.albumIds.filter((x) => L.albums[x].artistId === ar.id).sort((a, b) => L.albums[b].year - L.albums[a].year);
    const top = albums.flatMap((a) => L.albums[a].trackIds).slice(0, 10);
    const similar = L.artistIds.filter((x) => x !== ar.id && L.artists[x].tags[0] === ar.tags[0]).slice(0, 8);
    return {
      artist: ar, topTracks: top, counts: Object.fromEntries(top.map((t, i) => [t, { plays: 90_000 - i * 7000, listeners: 20_000 - i * 1500 }])), popularVia: 'lastfm',
      albums, discography: { albums: albums.filter((a) => L.albums[a].type === 'album'), singles: albums.filter((a) => L.albums[a].type !== 'album'), compilations: [], appearsOn: [] },
      similar, entities: entities({ tracks: top, albums, artists: [ar.id, ...similar] }),
    };
  }],
  ['GET', /^\/api\/tags\/([^/]+)$/, (m) => {
    const name = decodeURIComponent(m[1]);
    if (!L.tags.includes(name)) return err(404, 'not_found', 'No source knows this tag.');
    const artists = L.artistIds.filter((id) => L.artists[id].tags.includes(name)).slice(0, 12);
    const albums = L.albumIds.filter((id) => L.albums[id].tags.includes(name)).slice(0, 8);
    return { tag: { id: name, name, desc: `${cap(name)}: records the fixture library files under it.`, related: L.tags.filter((t) => t !== name).slice(0, 4) }, artists, newReleases: albums, entities: entities({ artists, albums }) };
  }],
  ['GET', /^\/api\/radio$/, (m, q) => {
    const exclude = new Set((q.get('exclude') || '').split(','));
    const items = L.trackIds.filter((id, i) => i % 13 === 4 && !exclude.has(id)).slice(0, 12);
    return { items, entities: entities({ tracks: items, albums: albumsOf(items) }) };
  }],
  ['GET', /^\/api\/lyrics\/([^/]+)$/, (m) => L.lyrics(m[1]) || err(404, 'not_found', 'No lyrics for this song.')],
  ['GET', /^\/api\/skins$/, () => ({ items: [], more: false })],
  ['GET', /^\/api\/skins\/favourites$/, () => ({ items: [] })],
];

function sortTracks(ids, sort) {
  const t = L.tracks;
  const by = {
    album: (a, b) => t[a].album.localeCompare(t[b].album) || t[a].no - t[b].no,
    title: (a, b) => t[a].title.localeCompare(t[b].title),
    artist: (a, b) => t[a].artist.localeCompare(t[b].artist) || t[a].album.localeCompare(t[b].album) || t[a].no - t[b].no,
    recent: (a, b) => L.songs[b].addedAt.localeCompare(L.songs[a].addedAt),
  }[sort];
  return by ? [...ids].sort(by) : ids;
}
function deepMerge(into, patch) {
  for (const [k, v] of Object.entries(patch)) {
    if (v && typeof v === 'object' && !Array.isArray(v) && into[k] && typeof into[k] === 'object') deepMerge(into[k], v);
    else into[k] = v;
  }
}
function cap(w) { return w[0].toUpperCase() + w.slice(1); }

function emit(topic, data) {
  const line = `event: ${topic}\ndata: ${JSON.stringify(data)}\n\n`;
  for (const res of clients) res.write(line);
}

// ---------- covers: a two-colour gradient per id, as a real PNG at the size asked. The 512 px one has a fine grain,
// so it weighs what a photo cover does (tens of KB): the desktop app hands that size to its mini player ----------
const art = new Map(), artPending = new Set(), artRequests = new Map();
function cover(id, size) {
  const key = id + '@' + size;
  if (art.has(key)) return art.get(key);
  let h = 0;
  for (const c of id) h = (Math.imul(h, 31) + c.charCodeAt(0)) >>> 0;
  const c1 = [h & 255, (h >> 8) & 255, (h >> 16) & 255], c2 = [255 - c1[1], c1[2], (c1[0] + 96) & 255];
  const row = 1 + size * 3, raw = Buffer.alloc(row * size);
  for (let y = 0; y < size; y++) {
    raw[y * row] = 0;
    for (let x = 0; x < size; x++) {
      const f = (x + y) / (2 * size), o = y * row + 1 + x * 3;
      const grain = size >= 512 ? ((x * 7919 + y * 104729) % 23) - 11 : 0;
      for (let k = 0; k < 3; k++) raw[o + k] = Math.max(0, Math.min(255, Math.round(c1[k] * (1 - f) + c2[k] * f) + grain));
    }
  }
  const png = Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', Buffer.from([...u32(size), ...u32(size), 8, 2, 0, 0, 0])),
    chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0)),
  ]);
  art.set(key, png);
  return png;
}
function u32(n) { return [(n >>> 24) & 255, (n >>> 16) & 255, (n >>> 8) & 255, n & 255]; }
const CRC = Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; return c >>> 0; });
function chunk(type, data) {
  const td = Buffer.concat([Buffer.from(type), data]);
  let c = 0xffffffff; for (const b of td) c = CRC[(c ^ b) & 255] ^ (c >>> 8);
  return Buffer.concat([Buffer.from(u32(data.length)), td, Buffer.from(u32((c ^ 0xffffffff) >>> 0))]);
}

// ---------- audio: a quiet 8 kHz 8-bit WAV as long as the song, with Range ----------
const RATE = 8000;
function wavHeader(n) {
  const b = Buffer.alloc(44);
  b.write('RIFF', 0); b.writeUInt32LE(36 + n, 4); b.write('WAVEfmt ', 8); b.writeUInt32LE(16, 16); b.writeUInt16LE(1, 20); b.writeUInt16LE(1, 22);
  b.writeUInt32LE(RATE, 24); b.writeUInt32LE(RATE, 28); b.writeUInt16LE(1, 32); b.writeUInt16LE(8, 34); b.write('data', 36); b.writeUInt32LE(n, 40);
  return b;
}
function audio(req, res, id, library) {
  const t = L.tracks[id];
  if (!t || (library && !t.saved)) return send(res, 404, { error: 'not_found', message: 'Not saved.' });
  if (t.albumId === L.failingAlbum) return send(res, 502, { error: 'upstream', message: 'No peer delivered this song.' });
  if (AUDIO_DIR) return audioFile(req, res, t);
  const n = t.duration * RATE, total = 44 + n;
  let start = 0, end = total - 1;
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  if (m) { start = m[1] ? +m[1] : 0; end = m[2] ? Math.min(+m[2], total - 1) : total - 1; }
  const head = wavHeader(n);
  const body = Buffer.alloc(end - start + 1, 128);
  if (start < 44) head.copy(body, 0, start, Math.min(44, end + 1));
  res.writeHead(m ? 206 : 200, { 'content-type': 'audio/wav', 'accept-ranges': 'bytes', 'content-length': body.length, ...(m ? { 'content-range': `bytes ${start}-${end}/${total}` } : {}) });
  res.end(body);
}

// FIXTURE_AUDIO_DIR=<dir> with song.mp3 and song.flac: songs alternate between the two real files (codecs and
// buffers as a listener's library has them) instead of the generated WAV
const AUDIO_DIR = process.env.FIXTURE_AUDIO_DIR;
const audioFiles = {};
async function audioFile(req, res, t) {
  const ext = t.no % 2 ? 'mp3' : 'flac';
  const body = (audioFiles[ext] ||= await readFile(join(AUDIO_DIR, 'song.' + ext)));
  const total = body.length, m = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  const start = m && m[1] ? +m[1] : 0, end = m && m[2] ? Math.min(+m[2], total - 1) : total - 1;
  res.writeHead(m ? 206 : 200, { 'content-type': ext === 'mp3' ? 'audio/mpeg' : 'audio/flac', 'accept-ranges': 'bytes', 'content-length': end - start + 1, ...(m ? { 'content-range': `bytes ${start}-${end}/${total}` } : {}) });
  res.end(body.subarray(start, end + 1));
}

// ---------- server ----------
function send(res, status, body) {
  if (status === 204) { res.writeHead(204); return res.end(); }
  const b = Buffer.from(JSON.stringify(body));
  res.writeHead(status, { 'content-type': 'application/json', 'content-length': b.length, 'cache-control': 'no-store' });
  res.end(b);
}

async function readBody(req) {
  const parts = [];
  for await (const p of req) parts.push(p);
  const s = Buffer.concat(parts).toString();
  try { return s ? JSON.parse(s) : undefined; } catch { return s; }
}

async function files(req, res, path) {
  const rel = normalize(decodeURIComponent(path === '/' ? '/index.html' : path)).replace(/^([/\\])+/, '');
  if (rel.startsWith('..')) return send(res, 404, { error: 'not_found', message: 'No such file.' });
  const f = join(WEB, rel);
  try {
    if (!(await stat(f)).isFile()) throw new Error('not a file');
    const body = await readFile(f);
    res.writeHead(200, { 'content-type': TYPES[extname(f)] || 'application/octet-stream', 'cache-control': 'no-cache', 'content-length': body.length });
    res.end(body);
  } catch {
    res.writeHead(404, { 'content-type': 'text/plain' }); res.end('not found');
  }
}

const server = createServer(async (req, res) => {
  const url = new URL(req.url, 'http://fixture');
  const path = url.pathname, q = url.searchParams;
  if (path === '/__fixture/emit' && req.method === 'POST') { const b = await readBody(req); emit(b.topic, b.data); return send(res, 204); }
  if (path === '/api/events') {
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-store', connection: 'keep-alive' });
    res.write(': open\n\n');
    clients.add(res); req.on('close', () => clients.delete(res));
    return;
  }
  let m;
  if (path === '/__fixture/art-requests') { const counts = Object.fromEntries(artRequests); artRequests.clear(); return send(res, 200, counts); }
  if (path === '/__fixture/art-pending' && req.method === 'POST') { (await readBody(req)).ids.forEach((id) => artPending.add(id)); return send(res, 204); }
  if (path === '/__fixture/art-ready' && req.method === 'POST') {
    const { ids } = await readBody(req); ids.forEach((id) => artPending.delete(id)); emit('art.ready', { keys: ids }); return send(res, 204);
  }
  if ((m = /^\/api\/(?:catalog\/)?art\/([^/]+)$/.exec(path))) {
    artRequests.set(url.pathname + url.search, (artRequests.get(url.pathname + url.search) || 0) + 1);
    // api.md /api/art: the sizes the hub serves; the fixture's pictures are 512 px at most, as a source with no more
    // is: never blown up, and X-Image-Size says so
    const asked = [96, 256, 512, 1024, 2048].includes(+q.get('size')) ? +q.get('size') : 256, size = Math.min(asked, 512);
    if (!L.albums[m[1]] && !L.artists[m[1]]) return send(res, 404, { error: 'not_found', message: 'No cover.' });
    if (artPending.has(m[1])) {
      res.writeHead(404, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      return res.end(JSON.stringify({ error: 'not_found', message: 'The picture is still on its way.' }));
    }
    const png = cover(m[1], size);
    res.writeHead(200, { 'content-type': 'image/png', 'content-length': png.length, 'cache-control': 'private, max-age=86400', 'x-image-size': size + 'x' + size });
    return res.end(png);
  }
  if ((m = /^\/api\/(library\/)?stream\/([^/]+)$/.exec(path)) && req.method === 'GET') return audio(req, res, m[2], !!m[1]);
  if ((m = /^\/api\/stream\/([^/]+)$/.exec(path)) && req.method === 'DELETE') return send(res, 204);
  if (path.startsWith('/api/') || path === '/health') {
    const body = req.method === 'GET' || req.method === 'HEAD' ? undefined : await readBody(req);
    for (const [method, re, fn] of routes) {
      if (method !== req.method) continue;
      const mm = re.exec(path);
      if (!mm) continue;
      const out = fn(mm, q, body);
      if (out && out.status && (out.body !== undefined || out.status === 204)) return send(res, out.status, out.body);
      return send(res, 200, out);
    }
    return send(res, 404, { error: 'not_found', message: `The fixture hub has no ${req.method} ${path}.` });
  }
  return files(req, res, path);
});
server.listen(PORT, '127.0.0.1', () => console.log(`fixture hub on http://127.0.0.1:${PORT}`));
