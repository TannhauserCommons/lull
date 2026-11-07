// The fixture hub: serves this checkout's web/ from disk and answers /api from library.mjs, so the UI
// gates (perf, screens, a11y) run anywhere with Node alone: no image, no Soulseek, no outside service,
// the same answers on every run. It is a stand-in for the hub's contract (docs/context/api.md), not a
// second hub: anything the UI asks that is not modelled here answers 404 `not_found`, which the specs
// count as a failure, so a new endpoint shows up as a gap to fill rather than as a silent blank.
//
//   node tests/ui/fixture-hub/server.mjs [port]      (default 8790; LULL_WEB=<dir> serves another web/)
//
// GET /api/themes lists the five built-in designs (web/designs/<id>/theme.json), then the listener's.
// FIXTURE_THEMES=<dir> serves the theme packages in that folder (<dir>/<id>/theme.json...) as the listener's own, as the
// hub stores them (docs/themes.md, section 1): GET /api/themes lists them with their rev and the pictures made of them
// (ADR-0080), /api/themes/<id>/<file> serves their files. POST /api/themes (a .lulltheme) and PUT /api/themes/<id>/<file>
// write into that folder and send `theme.changed`, unchecked (the hub's checks are crates/themes'). The pictures
// PUT /api/theme-pictures/<id>/<rev>-<mode>.webp keeps live in memory for the run; one of a rev that is no longer the
// theme's is a 409, as on the hub.
//
// Your skins (docs/context/skins.md): the skins the hub keeps start empty and live in memory for the run; POST
// /api/skins/mine?name= adds one (checked as below, one per md5), DELETE /api/skins/mine/<md5> removes one; both send
// `skins.mine`, as the hub does.
// FIXTURE_SKINS=<dir> is the skins folder the hub only reads (LULL_SKINS_DIR): its .wsz files are listed after the
// hub's (read again on every call, so POST /api/skins/mine/rescan answers the same), one per md5. /api/skins/<md5>.wsz
// serves either. A file is a skin when it is a zip under 16 MB naming main.bmp, a cruder check than the hub's
// (crates/skins/src/wsz.rs).
//
// POST /__fixture/emit { topic, data } sends one server event to every open /api/events stream (with X-Fixture-Client,
// to that client's streams only). POST /__fixture/drop-events ends that client's streams, as a hub restart or a dropped
// connection does: the browser connects again by itself.
// POST /__fixture/artist-late { id, pending, errors } makes that artist's page answer with those sections still coming
// (empty, named in `pending`) and `errors`, for that client; POST /__fixture/artist-in { id } answers it whole again
// and sends nothing: an `artist.updated` the page never heard.
// POST /__fixture/trickle { id, bytesPerS, seekAhead } makes that unsaved song a Soulseek transfer arriving at bytesPerS from
// now (its WAV is 8000 bytes a second of sound): GET /api/stream/<id> serves only what has arrived and holds a range that
// starts past it until it has, as the hub does, and `stream` events report the transfer twice a second until it is complete.
// With seekAhead, the transfer starts over where a request waits, as the hub's legs do (ADR-0014). With file (a path), the
// song's bytes are that file's, as the browser reads a real MP3, M4A or FLAC; GET /__fixture/reads?id= lists the offsets
// the browser asked for, with when.
// POST /__fixture/art-pending { ids } makes those covers answer 404 "not yet" (no-store), as the hub does while it
// fetches one; POST /__fixture/art-ready { ids } serves them again and sends `art.ready { keys }`.
// GET /__fixture/art-requests: cover requests that reached the hub since the last call, by URL (and resets them).
// The three art ones are per client: a browser context that sends `X-Fixture-Client: <id>` (and the calls that name
// the same id) sees only its own pending covers, art.ready events and counts, so gates running side by side on one
// hub never count or answer each other's pictures. With no header, the client is '' (one shared scope, as before).
//
// PUT /api/pictures/screen { gen, visible, near } (api.md) is kept per client, as the hub keeps it per device; a picture
// or size lull doesn't serve is a 400. POST /__fixture/art-cold { pace } makes every cover of that client cold, as on a
// hub that has fetched none: each answers 404 "not yet" until the hub has it. The hub then gets one every `pace` ms, in
// the order the hub's picture service works (ADR-0033): the client's latest screen, visible then near, top to bottom,
// then covers asked for that no screen shows, oldest first; each is announced with `art.ready`. GET /__fixture/art-filled
// lists the ids it got, in order. GET /__fixture/screen is the client's latest screen as reported ({ gen, visible,
// near }, raw), or null.

import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { appendFile, readFile, stat } from 'node:fs/promises';
import { deflateSync, inflateRawSync } from 'node:zlib';
import { dirname, extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as L from './library.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const WEB = process.env.LULL_WEB || join(here, '..', '..', '..', 'web');
const PORT = +(process.argv[2] || process.env.FIXTURE_PORT || 8790);
const THEMES = process.env.FIXTURE_THEMES || null;
const SKINS = process.env.FIXTURE_SKINS || null;
// FIXTURE_HUB_SLSKD=1: a hub that runs its own slskd (the desktop app's, the Linux package's), whose Soulseek account is
// two config keys the hub takes only together, as crates/core's Config does; it starts with none
const MANAGED = process.env.FIXTURE_HUB_SLSKD === '1';
const account = { user: null, pass: null };
function configItems() {
  return [
    { key: 'LULL_PUBLIC_URL', value: null, set: false, source: 'default', secret: false, restart: true },
    { key: 'LULL_SLSKD_URL', value: 'http://slskd:5030', set: true, source: 'env', secret: false, restart: true },
    { key: 'LULL_SLSKD_API_KEY', value: null, set: true, source: 'env', secret: true, restart: true },
    SKINS ? { key: 'LULL_SKINS_DIR', value: '/skins', set: true, source: 'env', secret: false, restart: true }
      : { key: 'LULL_SKINS_DIR', value: null, set: false, source: 'default', secret: false, restart: true },
    ...(MANAGED ? [
      { key: 'LULL_SOULSEEK_USERNAME', value: account.user, set: account.user != null, source: account.user != null ? 'stored' : 'default', secret: false, restart: true },
      { key: 'LULL_SOULSEEK_PASSWORD', value: null, set: account.pass != null, source: account.pass != null ? 'stored' : 'default', secret: true, restart: true },
    ] : []),
  ];
}
// The switches that are on, as the hub's LULL_FLAGS names them (ADR-0096): off unless a feature's own spec turns its own on.
// What a real hub's /health and /api/updates say it is (lull_core::build), fixed so the pictures don't move.
const BUILD = { release: '0.0.0-fixture', changes: 0, commit: null, install: 'docker' };
const FLAGS = (process.env.LULL_FLAGS || '').split(',').map((n) => n.trim()).filter(Boolean);

const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2', '.webmanifest': 'application/manifest+json',
  '.ico': 'image/x-icon', '.txt': 'text/plain', '.jpg': 'image/jpeg', '.wsz': 'application/zip',
  '.mp3': 'audio/mpeg', '.flac': 'audio/flac', '.m4a': 'audio/mp4', '.ogg': 'audio/ogg',
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
    library: { duplicatePolicy: 'ask', autoSaveLikes: false, musicFolder: { source: 'config', value: '/music' } },
    sharing: { uploadSlots: 3, uploadLimitKBs: 500, mode: { source: 'config', value: 'library' } },
    discovery: { adventurousness: 0.4, on: { listenbrainz: true, musicbrainz: true, lastfm: true } },
    cache: { capGB: { source: 'config', value: 10 }, usedGB: 1.2 },
    app: { version: '0.0.0-fixture', server: { source: 'config', value: 'http://127.0.0.1' } },
  },
  plays: 0,
};
const clients = new Map();   // open /api/events streams -> their fixture client

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

// FIXTURE_PLAYS_LOG=<file>: each play's start and end, one JSON line each, to compare players' listening history
function logPlay(kind, playId, body) {
  if (process.env.FIXTURE_PLAYS_LOG) appendFile(process.env.FIXTURE_PLAYS_LOG, JSON.stringify({ kind, playId, body }) + '\n').catch(() => {});
}

// ---------- routes: [method, regex, handler(match, query, body) -> { status?, body } | body] ----------
const routes = [
  ['GET', /^\/health$/, () => ({ version: '0.0.0-fixture', build: BUILD, services: SERVICES, flags: FLAGS })],
  ['GET', /^\/api\/updates$/, () => ({ version: '0.0.0-fixture', build: BUILD, state: 'current', latest: { version: '0.0.0-fixture', url: 'https://github.com/TannhauserCommons/lull/releases', publishedAt: null }, checkedAt: 0, error: null })],
  ['GET', /^\/api\/ping$/, () => ({ status: 204 })],
  ['GET', /^\/api\/setup$/, () => ({ claimed: true, soulseekManaged: MANAGED, steps: { library: true, soulseek: true, lastfm: true } })],
  ['GET', /^\/api\/devices$/, () => ({
    me: 'd-fixture',
    items: [
      { id: 'd-fixture', name: 'This browser', kind: 'browser', createdAt: L.NOW - 86400 * 30, lastSeen: L.NOW, private: false, online: true },
      { id: 'd-phone', name: 'Phone', kind: 'app', createdAt: L.NOW - 86400 * 20, lastSeen: L.NOW - 7200, private: false, online: false },
    ],
  })],
  ['PATCH', /^\/api\/devices\/me$/, () => ({ status: 204 })],
  ['POST', /^\/api\/devices\/me\/state$/, () => ({ status: 204 })],
  ['POST', /^\/api\/devices\/([^/]+)\/command$/, () => ({ status: 204 })],
  ['GET', /^\/api\/config$/, () => ({ items: configItems() })],
  // a hub that runs its own slskd keeps the Soulseek account as two config keys, taken only together
  ['PATCH', /^\/api\/config$/, (m, q, body) => {
    const user = body && body.LULL_SOULSEEK_USERNAME, pass = body && body.LULL_SOULSEEK_PASSWORD;
    if (!MANAGED || ((user == null) !== (pass == null) && !(user == null ? account.user : account.pass))) {
      return { status: 400, body: { error: 'bad_request', message: 'LULL_SOULSEEK_USERNAME and LULL_SOULSEEK_PASSWORD must be set together' } };
    }
    if (user != null) account.user = user;
    if (pass != null) account.pass = pass;
    return { items: configItems() };
  }],
  // a test's own changes (X-Fixture-Client) stay its own, over everyone's settings; the hub tells every device
  ['GET', /^\/api\/settings$/, (m, q, body, client) => settingsOf(client)],
  ['PATCH', /^\/api\/settings$/, (m, q, body, client) => {
    if (client) { const own = ownSettings.get(client) || {}; deepMerge(own, body || {}); ownSettings.set(client, own); } else deepMerge(state.settings, body || {});
    const now = settingsOf(client); if (client) emit('settings', now, client); return now;
  }],
  // every fixture song has its album pictured, so each known id resolves at once and none is due a lookup
  ['POST', /^\/api\/catalog\/resolve$/, (m, q, body) => {
    const known = ((body && body.trackIds) || []).filter((id) => L.tracks[id]).slice(0, 200);
    return { entities: entities({ tracks: known, albums: albumsOf(known) }) };
  }],
  ['GET', /^\/api\/themes$/, () => ({ items: [...builtInThemes(), ...(THEMES ? themeItems() : [])] })],
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
  // liked songs the library doesn't hold, and saving them (ADR-0099): what a test asked is kept as its own
  ['GET', /^\/api\/likes\/unsaved$/, (m, q, body, client) => { const items = unsavedLikes(client); return { items, total: items.length }; }],
  ['POST', /^\/api\/likes\/save$/, (m, q, body, client) => {
    const wanted = body && body.trackIds, ids = unsavedLikes(client).filter((id) => !wanted || wanted.includes(id));
    likeLog(client).saved.push(...ids); return { asked: ids.length };
  }],
  ['PUT', /^\/api\/likes\/(.+)$/, (m, q, body, client) => {
    // a test's own likes stay its own; without a scope, everyone's
    if (client) likeLog(client).likes.push({ trackId: m[1], batch: q.get('batch') === 'true' });
    else if (!state.likes.includes(m[1])) state.likes.unshift(m[1]);
    emit('like', { trackId: m[1], liked: true }, client || undefined); return { status: 204 };
  }],
  ['DELETE', /^\/api\/likes\/(.+)$/, (m) => { state.likes = state.likes.filter((t) => t !== m[1]); emit('like', { trackId: m[1], liked: false }); return { status: 204 }; }],
  // With the namesakes switch, the follow of Soft Choir is one from before that nothing says which of (ADR-0035).
  ['GET', /^\/api\/follows$/, () => ({ ...state.follows, entities: entities({ artists: state.follows.artists }),
    ...(FLAGS.includes('namesakes') ? { unsure: state.follows.artists.filter((id) => L.artists[id].name === 'Soft Choir') } : {}) })],
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
  ['POST', /^\/api\/plays$/, (m, q, body) => { const playId = String(++state.plays); logPlay('start', playId, body); return { playId }; }],
  ['PATCH', /^\/api\/plays\/(.+)$/, (m, q, body) => { logPlay('end', m[1], body); return { playId: m[1] }; }],
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
  // A trickling song has two copies (ADR-0015): one the peer says is fast that lull never measured, one lull measured
  // yesterday; any other song has none, so the gates' screens stay as they are.
  ['GET', /^\/api\/tracks\/([^/]+)\/sources$/, (m) => {
    const sources = trickles.has(m[1]) ? trickledSources(m[1]) : [], youtube = youtubeCopy(m[1]);
    // the hub's own sentence for an empty answer (soulseek api.rs, NO_PEER)
    return { sources, youtube, done: true, why: !sources.length && !youtube ? 'No Soulseek peer shares this song right now.' : null };
  }],
  ['PUT', /^\/api\/stream\/prefetch$/, () => ({ status: 204 })],

  // discovery
  // Refresh picks: a whole new shelf "now" (the fixture's feed is the same), announced a moment later; again within the
  // hour, not
  ['POST', /^\/api\/feed\/refresh$/, (m, q, body, client) => {
    const last = refreshedAt.get(client), now = Date.now();
    if (last && now - last < 3600000) return { building: false, retryInS: Math.ceil((3600000 - (now - last)) / 1000) };
    refreshedAt.set(client, now);
    setTimeout(() => emit('feed.updated', { updated: L.NOW }, client || undefined), 300);
    return { building: true };
  }],
  ['GET', /^\/api\/feed$/, (m, q, body, client) => {
    const picks = L.trackIds.filter((_, i) => i % 97 === 1).slice(0, 12);
    const fresh = L.albumIds.slice(-12, -2).reverse();   // the catalog's albums, none saved
    const feed = {
      updated: L.NOW - 600, summary: 'Dreamy guitars and late synths this week.',
      newReleases: fresh,
      picks: picks.map((trackId, i) => ({ trackId, reasons: [i % 2 ? { kind: 'tag', name: L.tracks[trackId].tags[0] } : { kind: 'like', name: L.tracks[trackId].artist, ref: L.tracks[trackId].artistId }], via: 'listenbrainz' })),
      radioStarters: L.artistIds.slice(0, 6).map((id) => ({ id: 'rs-' + id, title: `${L.artists[id].name} radio`, sub: L.artists[id].tags.join(', '), seed: { kind: 'artist', id } })),
      jumpBackIn: [...L.albumIds.slice(0, 4).map((id) => ({ kind: 'album', id })), ...L.artistIds.slice(6, 10).map((id) => ({ kind: 'artist', id }))],
      followedTags: state.follows.tags, coldStartTags: L.tags.slice(0, 8),
      entities: entities({ tracks: picks, albums: [...fresh, ...L.albumIds.slice(0, 4), ...albumsOf(picks)], artists: L.artistIds.slice(0, 10) }),
    };
    // FIXTURE_FEED_SHAPE: every client's feed in that shape (themes:check of a state the plain feed doesn't show)
    const shape = feedShapes.get(client) || process.env.FIXTURE_FEED_SHAPE;
    // a first run: nothing known of the listener, the starter tags' picks and their artists
    if (shape === 'cold') return { ...feed, cold: true, followedTags: [], jumpBackIn: [], newReleases: [], coldStartArtists: L.artistIds.slice(0, 6) };
    // where the first picks can play from, as the hub's checks found (one with no copy anywhere)
    if (shape === 'avail') {
      const marks = [{ avail: ['soulseek'] }, { avail: ['soulseek', 'youtube'] }, { avail: ['youtube'] }, { noSource: true }];
      return { ...feed, picks: feed.picks.map((p, i) => ({ ...p, ...(marks[i] || {}) })) };
    }
    // each followed tag with the tags that come with it on this month's releases ("you might like")
    if (shape === 'related') {
      const others = L.tags.filter((t) => !feed.followedTags.includes(t));
      // the first pick by an artist not followed, its reason with what the listener did, as the hub counts it
      const at = feed.picks.findIndex((p) => p.reasons[0].kind === 'like' && !state.follows.artists.includes(p.reasons[0].ref));
      const picks = feed.picks.map((p, i) => (i !== at ? p : { ...p, reasons: [{ ...p.reasons[0], evidence: { plays: 14 } }] }));
      return { ...feed, picks, entities: { ...feed.entities, tags: feed.followedTags.map((t, i) => ({ id: t, name: t, related: others.slice(i * 2, i * 2 + 2) })) } };
    }
    return feed;
  }],
  ['GET', /^\/api\/search$/, (m, q) => {
    const w = words(q.get('q'));
    if (!w) return err(400, 'bad_request', 'Search for some words.');
    const tracks = L.trackIds.filter((id) => has(`${L.tracks[id].title} ${L.tracks[id].artist}`, w)).slice(0, 50);
    const albums = L.albumIds.filter((id) => has(`${L.albums[id].title} ${L.albums[id].artist}`, w)).slice(0, 20);
    const artists = L.artistIds.filter((id) => has(L.artists[id].name, w)).slice(0, 20);
    const top = artists[0] ? { kind: 'artist', id: artists[0] } : albums[0] ? { kind: 'album', id: albums[0] } : tracks[0] ? { kind: 'track', id: tracks[0] } : null;
    const found = { query: w, top, artists, albums, tracks, playlists: [], pending: [], entities: entities({ tracks, albums: [...albums, ...albumsOf(tracks)], artists }) };
    // With the namesakes switch, Soft Choir's namesakes are rows of their own, each told apart (ADR-0035).
    if (FLAGS.includes('namesakes') && artists.some((id) => L.artists[id].name === 'Soft Choir')) {
      const about = namedPage('Soft Choir').about;
      for (const n of NAMESAKES) { found.artists.push(n.id); found.entities.artists[n.id] = { ...n, about: about[n.id] }; }
      const own = artists.find((id) => L.artists[id].name === 'Soft Choir');
      found.entities.artists[own] = { ...found.entities.artists[own], about: '1990s English dream pop band' };
    }
    return found;
  }],
  ['GET', /^\/api\/soulseek\/search$/, (m, q) => {
    const searchId = 'srch-' + words(q.get('q')).replace(/\W+/g, '-');
    setTimeout(() => emit('search.results', { searchId, query: words(q.get('q')), files: 0, peers: 0, done: true, results: [], entities: {}, onlySoulseek: [] }), 50);
    return { searchId, soulseek: null, entities: {} };
  }],
  // A share host's /1/ link to a song or an album, read by its names (the hub reads every kind of link: discovery's
  // open.rs); anything else is a 404, as the hub answers a link to something it can't find.
  ['GET', /^\/api\/open$/, (m, q) => {
    let u;
    try { u = new URL(q.get('link') || ''); } catch { return err(400, 'bad_request', 'That isn\'t a link.'); }
    const [v, kind, artist, title] = u.pathname.split('/').slice(1).map(decodeURIComponent);
    const named = (x) => x.artist === artist && x.title === title;
    const track = v === '1' && kind === 'song' && L.trackIds.find((t) => named(L.tracks[t]));
    if (track) return { kind: 'track', id: track, entities: entities({ tracks: [track], albums: albumsOf([track]) }) };
    const album = v === '1' && kind === 'album' && L.albumIds.find((a) => named(L.albums[a]));
    if (album) return { kind: 'album', id: album, entities: entities({ albums: [album] }) };
    return err(404, 'not_found', 'That song, album or artist isn\'t there any more.');
  }],
  ['GET', /^\/api\/albums\/([^/]+)$/, (m) => {
    const a = L.albums[m[1]]; if (!a) return err(404, 'not_found', 'lull doesn\'t know this album.');
    return { album: a, entities: entities({ tracks: a.trackIds, albums: [a.id, ...L.albumIds.filter((x) => L.albums[x].artistId === a.artistId)], artists: [a.artistId] }) };
  }],
  // ADR-0035's name page (the `namesakes` switch): Soft Choir shares its name with two invented artists.
  ['GET', /^\/api\/artists\/named$/, (m, q) => FLAGS.includes('namesakes') ? namedPage(q.get('name')) : err(404, 'not_found', 'No such page.')],
  ['POST', /^\/api\/artists\/(named\/split|place|named\/choose)$/, (m, q, body) => FLAGS.includes('namesakes')
    ? { artist: body.artist || 'ar-5c0f7c401c0a0003' } : err(404, 'not_found', 'No such page.')],
  ['GET', /^\/api\/artists\/([^/]+)$/, (m, q, body, client) => {
    const ar = L.artists[m[1]]; if (!ar) return err(404, 'not_found', 'lull doesn\'t know this artist.');
    const page = artistPage(ar), late = artistLate.get(client + ' ' + ar.id);
    return late ? lateArtist(page, late) : page;
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
  ['GET', /^\/api\/skins\/mine$/, () => mineSkins()],
  ['POST', /^\/api\/skins\/mine\/rescan$/, () => mineSkins()],
  ['DELETE', /^\/api\/skins\/mine\/([0-9a-f]{32})$/, (m) => {
    if (ownSkins.delete(m[1])) { emit('skins.mine', {}); return { status: 204 }; }
    const there = mineSkins().items.find((i) => i.md5 === m[1]);
    return there ? err(409, 'conflict', `${there.file} is in your skins folder /skins, which lull only reads: remove it there.`)
      : err(404, 'not_found', `None of your skins is ${m[1]}.`);
  }],
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

/* GET /api/artists/named?name=: Soft Choir (the listener's, with an album, two songs nothing places and a follow from
   before nothing says which of) and two
   invented namesakes, one described by MusicBrainz's comment and one by its type, country and years */
const NAMESAKES = [
  { id: 'ar-5c0f7c401c0a0001', name: 'Soft Choir', tags: [], listeners: null, bio: null, musicbrainzId: null },
  { id: 'ar-5c0f7c401c0a0002', name: 'Soft Choir', tags: [], listeners: null, bio: null, musicbrainzId: null },
];
function namedPage(name) {
  const own = L.artistIds.find((id) => L.artists[id].name === name);
  if (!own) return { name, artists: [], about: {}, albums: {}, yours: [], unplaced: [], entities: { tracks: {}, albums: {}, artists: {} } };
  const albums = L.albumIds.filter((a) => L.albums[a].artistId === own);
  const unplaced = (L.albums[albums[1] || albums[0]]?.trackIds || []).slice(0, 2);
  const e = entities({ tracks: unplaced, albums: albums.slice(0, 1), artists: [own] });
  for (const n of NAMESAKES) e.artists[n.id] = n;
  return {
    name, artists: [own, ...NAMESAKES.map((n) => n.id)],
    about: { [NAMESAKES[0].id]: '1970s Welsh male voice choir', [NAMESAKES[1].id]: 'Group · NZ · since 2019' },
    albums: albums[0] ? { [own]: albums[0] } : {}, yours: [own], unplaced, askFollow: true, entities: e,
  };
}

/* GET /api/artists/{id}: the whole page */
function artistPage(ar) {
  const albums = L.albumIds.filter((x) => L.albums[x].artistId === ar.id).sort((a, b) => L.albums[b].year - L.albums[a].year);
  const top = albums.flatMap((a) => L.albums[a].trackIds).slice(0, 10);
  const similar = L.artistIds.filter((x) => x !== ar.id && L.artists[x].tags[0] === ar.tags[0]).slice(0, 8);
  return {
    artist: ar, topTracks: top, counts: Object.fromEntries(top.map((t, i) => [t, { plays: 90_000 - i * 7000, listeners: 20_000 - i * 1500 }])), popularVia: 'lastfm',
    albums, discography: { albums: albums.filter((a) => L.albums[a].type === 'album'), singles: albums.filter((a) => L.albums[a].type !== 'album'), compilations: [], appearsOn: [] },
    similar, entities: entities({ tracks: top, albums, artists: [ar.id, ...similar] }),
  };
}
/* the page with `late.pending` sections still coming (empty) and `late.errors` failed, as the hub answers a first visit */
function lateArtist(page, late) {
  const out = { ...page, pending: late.pending || [], errors: late.errors || {} };
  const gone = new Set([...out.pending, ...Object.keys(out.errors)]);
  if (gone.has('albums')) { out.albums = []; out.discography = { albums: [], singles: [], compilations: [], appearsOn: [] }; }
  if (gone.has('topTracks')) { out.topTracks = []; out.counts = {}; }
  if (gone.has('similar')) out.similar = [];
  return out;
}
const artistLate = new Map();   // client + ' ' + artist id -> { pending, errors }

function emit(topic, data, only) {
  const line = `event: ${topic}\ndata: ${JSON.stringify(data)}\n\n`;
  for (const [res, client] of clients) if (only === undefined || client === only) res.write(line);
}
const clientOf = (req) => req.headers['x-fixture-client'] || '';
const ownSettings = new Map();   // fixture client -> its own settings changes
function settingsOf(client) { const out = structuredClone(state.settings); if (ownSettings.has(client)) deepMerge(out, structuredClone(ownSettings.get(client))); return out; }
const likeLogs = new Map();      // fixture client -> { likes: [{ trackId, batch }], saved: [ids asked to save] }
function likeLog(client) { if (!likeLogs.has(client)) likeLogs.set(client, { likes: [], saved: [] }); return likeLogs.get(client); }
const unsavedLikes = (client) => [...new Set([...likeLog(client).likes.map((l) => l.trackId), ...state.likes])]
  .filter((id) => !L.savedIds.includes(id) && !likeLog(client).saved.includes(id));
const feedShapes = new Map();    // fixture client -> 'cold' | 'related': the feed it is served
const refreshedAt = new Map();   // fixture client -> when its Refresh last built (ms)
/* a comment on every open stream every 15 s, as the hub's keep-alive (crates/server/src/events.rs): a client that
   stopped listening finds out at the next one */
setInterval(() => { for (const res of clients.keys()) res.write(': keep-alive\n\n'); }, 15000).unref();

// ---------- the screen each client reports, and covers a cold hub gets in that order (see the top) ----------
const screens = new Map(), colds = new Map();   // client -> { visible: [id], near: [id] }; client -> cold state
const SIZES = [96, 256, 512, 1024, 2048];
/* a reported picture's id, or null when lull serves no such picture at that size (api.md PUT /api/pictures/screen) */
function shownId(p) {
  const m = Array.isArray(p) && typeof p[0] === 'string' && /^(catalog\/art\/(?:al|ar|pls)|art\/al)-[0-9a-f]+$/.exec(p[0]);
  if (!m || !(SIZES.includes(p[1]) || (p[1] === 0 && p[0].startsWith('catalog/art/ar-')))) return null;
  return p[0].slice(p[0].lastIndexOf('/') + 1);
}
function report(client, body) {
  const lists = [body && body.visible || [], body && body.near || []];
  if (!body || typeof body.gen !== 'number' || !lists.every(Array.isArray)) return { status: 400, body: { error: 'bad_request', message: 'Send { gen, visible, near }.' } };
  const [visible, near] = lists.map((l) => l.map(shownId));
  if ([...visible, ...near].includes(null)) return { status: 400, body: { error: 'bad_request', message: 'Not a picture lull serves.' } };
  const was = screens.get(client);
  if (!was || body.gen > was.gen) screens.set(client, { gen: body.gen, visible, near, raw: body });
  return { status: 204 };
}
function goCold(client, pace) {
  const was = colds.get(client); if (was) clearInterval(was.timer);
  const c = { warm: new Set(), asked: [], filled: [] };
  c.timer = setInterval(() => {
    const s = screens.get(client) || { visible: [], near: [] };
    const next = [...s.visible, ...s.near, ...c.asked].find((id) => !c.warm.has(id) && (L.albums[id] || L.artists[id]));
    if (!next) return;
    c.warm.add(next); c.filled.push(next);
    emit('art.ready', { keys: [next] }, client);
  }, pace);
  colds.set(client, c);
}
/* a cover a cold hub hasn't got yet: noted as asked, answered "not yet" */
function cold(client, id) {
  const c = colds.get(client);
  if (!c || c.warm.has(id)) return false;
  if (!c.asked.includes(id)) c.asked.push(id);
  return true;
}

// ---------- covers: a two-colour gradient per id, as a real PNG at the size asked. The 512 px one has a fine grain,
// so it weighs what a photo cover does (tens of KB): the desktop app hands that size to its mini player ----------
const art = new Map(), artPending = new Set(), artRequests = new Map();   // pending: client + ' ' + id; requests: client -> url -> n
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

// The hub's own reason for a song no peer delivers, YouTube's part included (fetch.rs).
const FAIL_REASON = 'no peer shares this song right now; YouTube: no YouTube upload matches this song';
/* POST /api/stream/<id>/output-error { jobId }: the job's state answers (soulseek/src/fetch.rs, Fetcher::output_error) */
function outputError(res, id, jobId) {
  const t = L.tracks[id];
  jobId = jobId || (t && t.albumId === L.failingAlbum ? 'fixture-fail-' : 'fixture-broken-') + id;
  if (!t) return send(res, 404, { error: 'not_found', message: 'The hub no longer knows that job.' });
  if (t.albumId === L.failingAlbum) return send(res, 200, { trackId: id, jobId, status: 'failed', attempt: 0, error: FAIL_REASON });
  return send(res, 200, { trackId: id, jobId, status: 'streaming', attempt: 0, error: null, unplayable: "this copy can't be played in this player (WAV 8 kHz)" });
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
  // As the hub: the song's job sends a stream event when it opens, and its outcome is the job's (POST .../output-error).
  // Its event goes to the page that asked (X-Fixture-Client): pages playing the same song at once don't hear each other's.
  if (t.albumId === L.failingAlbum) {
    emit('stream', { trackId: id, jobId: 'fixture-fail-' + id, status: 'finding', attempt: 0, peersFound: 0, received: 0, size: 0 }, clientOf(req));
    return send(res, 502, { error: 'upstream', message: FAIL_REASON });
  }
  if (t.albumId === L.brokenAlbum && !library) {
    emit('stream', { trackId: id, jobId: 'fixture-broken-' + id, status: 'streaming', attempt: 0, peer: 'fixture-peer', format: 'WAV 8 kHz', received: 4096, size: 4096 }, clientOf(req));
    const junk = Buffer.alloc(4096, 0x5a);
    res.writeHead(200, { 'content-type': 'audio/wav', 'accept-ranges': 'bytes', 'content-length': junk.length });
    return res.end(junk);
  }
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
// buffers as a listener's library has them) instead of the generated WAV. With song.webm (Opus) and song.m4a (AAC)
// there too, YouTube's two formats join the turn: mp3, flac, webm, m4a.
const AUDIO_DIR = process.env.FIXTURE_AUDIO_DIR;
const AUDIO_TYPES = { mp3: 'audio/mpeg', flac: 'audio/flac', webm: 'audio/webm', m4a: 'audio/mp4' };
const audioFiles = {};
let audioTurn = null;
async function audioFile(req, res, t) {
  audioTurn ||= ['mp3', 'flac'].concat(await Promise.all(['webm', 'm4a'].map((x) => stat(join(AUDIO_DIR, 'song.' + x)).then(() => x, () => null))).then((x) => x.filter(Boolean)));
  const ext = audioTurn[(t.no + 1) % audioTurn.length];
  const body = (audioFiles[ext] ||= await readFile(join(AUDIO_DIR, 'song.' + ext)));
  const total = body.length, m = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  const start = m && m[1] ? +m[1] : 0, end = m && m[2] ? Math.min(+m[2], total - 1) : total - 1;
  res.writeHead(m ? 206 : 200, { 'content-type': AUDIO_TYPES[ext], 'accept-ranges': 'bytes', 'content-length': end - start + 1, ...(m ? { 'content-range': `bytes ${start}-${end}/${total}` } : {}) });
  res.end(body.subarray(start, end + 1));
}

// ---------- a Soulseek transfer arriving slowly (POST /__fixture/trickle) ----------
// The song's YouTube copy while Settings › Sources › YouTube is on, as the hub lists it beside the peers.
function youtubeCopy(id) {
  if (state.settings.sources.youtube.mode === 'off' || !L.tracks[id]) return null;
  return { peer: 'YouTube', path: `youtube:fixture${id.replace(/[^A-Za-z0-9]/g, '')}:251.webm`, sizeBytes: 4_100_000, format: 'YouTube · Opus 160', official: true };
}
function trickledSources(id) {
  const t = L.tracks[id], size = trickles.get(id).size;
  const row = (peer, speed, measuredAt, score) => ({
    peer, path: `@@music\\${t.artist}\\${t.album}\\${t.title}.wav`, format: 'WAV 8 kHz', sizeBytes: size, speed, speedMeasured: measuredAt != null,
    measuredAt, expectedSpeed: measuredAt != null ? speed : speed / 4, freeSlot: true, queue: 0, score, factors: { format: 30, speed: score - 60, slot: 15, queue: 15, stalled: 0 },
    preferred: false, stalledBefore: false, meets: true, queueTooLong: false, aboveMax: false,
  });
  return [row('fixture-peer', 1.2, L.NOW - 86400, 71), row('says-fast', 8.6, null, 70)];
}
// The transfer runs in legs as the hub's does (ADR-0014): one from the first byte missing; with `seekAhead`, a request
// waiting 300 ms for a byte more than JUMP_S of transfer away starts a leg there, and the gap fills after it.
const JUMP_S = 30;
const trickles = new Map();   // trackId -> { rate (bytes/s), size, parts [[from, to)], front, seekAhead, asks Map, timer }
const arrived = (tr, pos) => tr.parts.some(([a, b]) => a <= pos && pos < b);
const endFrom = (tr, pos) => (tr.parts.find(([a, b]) => a <= pos && pos < b) || [0, pos])[1];
function addPart(tr, a, b) {
  const all = [...tr.parts, [a, b]].sort((x, y) => x[0] - y[0]), merged = [];
  for (const r of all) { const last = merged.at(-1); if (last && r[0] <= last[1]) last[1] = Math.max(last[1], r[1]); else merged.push([...r]); }
  tr.parts = merged;
}
function firstMissing(tr) { const p = tr.parts[0]; return p && p[0] === 0 ? (p[1] < tr.size ? p[1] : null) : 0; }
function step(tr, ms) {
  const asked = [...tr.asks.values()].at(-1);
  if (tr.seekAhead && asked && Date.now() - asked.since > 300 && !arrived(tr, asked.at)
      && !(asked.at >= tr.front && (asked.at - tr.front) / tr.rate <= JUMP_S)) tr.front = asked.at;   // a jump
  if (tr.front == null) return;
  const to = Math.min(tr.size, tr.front + Math.round((tr.rate * ms) / 1000));
  const stop = tr.parts.map(([a]) => a).filter((a) => a > tr.front).sort((x, y) => x - y)[0] ?? tr.size;   // a leg stops at what arrived
  addPart(tr, tr.front, Math.min(to, stop));
  tr.front = Math.min(to, stop) >= stop ? firstMissing(tr) : to;
}
function trickle(id, rate, seekAhead, file) {
  const t = L.tracks[id];
  if (!t || t.saved) return { status: 400, body: { error: 'bad_request', message: 'Trickle an unsaved song.' } };
  clearInterval(trickles.get(id)?.timer);
  const bytes = file ? readFileSync(file) : null;
  const tr = { start: Date.now(), rate, size: bytes ? bytes.length : 44 + t.duration * RATE, bytes, type: file ? TYPES[extname(file)] : 'audio/wav',
    parts: [], front: 0, seekAhead: !!seekAhead, asks: new Map(), n: 0, reads: [] };
  let ticks = 0;
  tr.timer = setInterval(() => {
    step(tr, 100);
    if (++ticks % 5 && tr.front != null) return;
    const received = tr.parts.reduce((n, [a, b]) => n + b - a, 0);
    emit('stream', { trackId: id, jobId: 'fixture-' + id + '-' + tr.start, status: 'streaming', peer: 'fixture-peer', format: 'WAV 8 kHz',
      peersFound: 4, queuePos: 0, received, size: tr.size, ranges: tr.parts.map(([a, b]) => [a / tr.size, b / tr.size]),
      speed: tr.front == null ? 0 : rate, seekAheadS: tr.seekAhead ? JUMP_S : null, attempt: 0, failedPeers: [], error: null,
      belowMinimum: false, youtube: false });
    if (tr.front == null) clearInterval(tr.timer);
  }, 100);
  trickles.set(id, tr);
  return { status: 204 };
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function trickleAudio(req, res, id) {
  const tr = trickles.get(id), head = tr.bytes ? null : wavHeader(tr.size - 44);
  const m = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  const start = m && m[1] ? +m[1] : 0, end = m && m[2] ? Math.min(+m[2], tr.size - 1) : tr.size - 1;
  tr.reads.push([start, Date.now() - tr.start]);
  if (start >= tr.size) { res.writeHead(416, { 'content-range': `bytes */${tr.size}` }); return res.end(); }
  let gone = false;
  const ask = (at) => { const k = ++tr.n; tr.asks.set(k, { at, since: Date.now() }); return () => tr.asks.delete(k); };
  req.on('close', () => { gone = true; });
  if (!arrived(tr, start)) {   // a range ahead of the transfer waits for its bytes, asking for them
    const done = ask(start);
    while (!gone && !arrived(tr, start)) await sleep(100);
    done();
  }
  if (gone) return;
  res.writeHead(m ? 206 : 200, { 'content-type': tr.type, 'accept-ranges': 'bytes', 'content-length': end - start + 1, 'cache-control': 'no-store',
    ...(m ? { 'content-range': `bytes ${start}-${end}/${tr.size}` } : {}) });
  for (let pos = start; pos <= end && !gone;) {
    const to = Math.min(end + 1, endFrom(tr, pos));
    if (to > pos) {
      const chunk = tr.bytes ? tr.bytes.subarray(pos, to) : Buffer.alloc(to - pos, 128);
      if (head && pos < 44) head.copy(chunk, 0, pos, Math.min(44, to));
      res.write(chunk); pos = to;
    } else {
      const done = ask(pos);
      while (!gone && !arrived(tr, pos)) await sleep(100);
      done();
    }
  }
  res.end();
}

// ---------- server ----------
/* the skins the hub keeps: md5 -> { name, file, bytes }, newest last */
const ownSkins = new Map();
/* why bytes aren't a classic skin, or null */
function notSkin(bytes) {
  if (bytes.length > 16 * 1024 * 1024) return 'it is over 16 MB, which no classic skin is';
  if (bytes.subarray(0, 2).toString() !== 'PK') return 'the file is not a zip archive';
  if (!/main\.bmp/i.test(bytes.toString('latin1'))) return 'the archive has no main.bmp, so it is not a classic Winamp skin';
  return null;
}
/* POST /api/skins/mine?name=: 201 { added, item }, 200 when the listener has it already, 400 for a non-skin */
function addSkin(name, bytes) {
  const file = (name.split(/[\\/]/).pop().replace(/\.wsz$/i, '').replace(/[<>:"|?*]/g, '').trim() || 'Skin') + '.wsz';
  const why = notSkin(bytes);
  if (why) return err(400, 'bad_request', `${file} isn't a classic skin: ${why}.`);
  const md5 = createHash('md5').update(bytes).digest('hex');
  const had = mineSkins().items.find((i) => i.md5 === md5);
  if (had) return { status: 200, body: { added: false, item: had } };
  ownSkins.set(md5, { file, bytes });
  emit('skins.mine', {});
  return { status: 201, body: { added: true, item: mineSkins().items.find((i) => i.md5 === md5) } };
}
/* FIXTURE_SKINS as GET /api/skins/mine answers, and each skin's file by md5 */
function readSkins() {
  const files = readdirSync(SKINS, { recursive: true, withFileTypes: true })
    .filter((d) => d.isFile() && /\.wsz$/i.test(d.name))
    .map((d) => join(d.parentPath ?? d.path, d.name))
    .map((f) => ({ file: f, path: f.slice(SKINS.length + 1).replaceAll('\\', '/') }))
    .sort((a, b) => (a.path < b.path ? -1 : 1));
  const items = [], skipped = [], byMd5 = new Map();
  for (const { file, path } of files) {
    const bytes = readFileSync(file), name = path.split('/').pop(), why = notSkin(bytes);
    if (why) { skipped.push({ path, why, source: 'folder' }); continue; }
    const md5 = createHash('md5').update(bytes).digest('hex');
    if (byMd5.has(md5)) continue;
    byMd5.set(md5, file);
    items.push({ md5, name: name.replace(/\.wsz$/i, '').replaceAll('_', ' ').trim(), file: name, path, size: bytes.length, source: 'folder' });
  }
  return { items, skipped, byMd5 };
}
function mineSkins() {
  const own = [...ownSkins].map(([md5, s]) => ({ md5, name: s.file.replace(/\.wsz$/i, '').replaceAll('_', ' ').trim(), file: s.file, path: s.file, size: s.bytes.length, source: 'hub' }))
    .sort((a, b) => (a.path < b.path ? -1 : 1));
  const folder = SKINS ? readSkins() : { items: [], skipped: [] }, seen = new Set(own.map((i) => i.md5));
  return { hub: '/data/skins/mine', folder: SKINS ? '/skins' : null, watching: !!SKINS, problem: null,
    items: [...own, ...folder.items.filter((i) => !seen.has(i.md5))], skipped: folder.skipped };
}

const BUILT_IN = ['lull', 'skydeck', 'starlight', 'petal', 'glasshouse'];
function builtInThemes() {
  return BUILT_IN.map((id) => ({ ...JSON.parse(readFileSync(join(WEB, 'designs', id, 'theme.json'), 'utf8')), builtIn: true }));
}

/* FIXTURE_THEMES' packages as GET /api/themes lists the hub's: each manifest with the slots its files have, its rev and
   the modes a picture was made of for that rev */
function themeItems() {
  return readdirSync(THEMES, { withFileTypes: true }).filter((d) => d.isDirectory() && !d.name.startsWith('.')).flatMap((d) => {
    const item = themeItem(d.name);
    return item ? [item] : [];
  });
}
function themeItem(id) {
  const dir = join(THEMES, id);
  try {
    const m = JSON.parse(readFileSync(join(dir, 'theme.json'), 'utf8'));
    const slots = existsSync(join(dir, 'slots')) ? readdirSync(join(dir, 'slots')).filter((f) => f.endsWith('.html')).map((f) => f.slice(0, -5)) : [];
    const rev = themeRev(id);
    return { ...m, slots, builtIn: false, rev, pictures: ['day', 'night'].filter((mode) => themePictures.has(`${id}/${rev}-${mode}`)) };
  } catch { return null; }
}
/* the hub's rev: the theme's files' names, sizes and modification times (crates/themes, watch.rs and pictures.rs) */
function themeRev(id) {
  const h = createHash('sha1');
  (function walk(dir) {
    for (const name of readdirSync(dir).sort()) {
      const f = join(dir, name), st = statSync(f);
      h.update(name);
      if (st.isDirectory()) walk(f); else h.update(`${st.size}:${st.mtimeMs}`);
    }
  })(join(THEMES, id));
  return h.digest('hex').slice(0, 16);
}
const themePictures = new Map();                  // '<id>/<rev>-<mode>' -> the WebP a page sent

function writeTheme(id, files) {
  for (const [path, bytes] of files) {
    const f = join(THEMES, id, normalize(path).replace(/^([/\\])+/, ''));
    mkdirSync(dirname(f), { recursive: true });
    writeFileSync(f, bytes);
  }
  emit('theme.changed', { id });
  return themeItem(id);
}
/* a .lulltheme's files: a zip's stored and deflated entries, at its root or inside one folder */
function unzip(buf) {
  const end = buf.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (end < 0) return null;
  const files = new Map();
  for (let i = 0, at = buf.readUInt32LE(end + 16); i < buf.readUInt16LE(end + 10); i++) {
    const method = buf.readUInt16LE(at + 10), size = buf.readUInt32LE(at + 20), nameLen = buf.readUInt16LE(at + 28);
    const name = buf.toString('utf8', at + 46, at + 46 + nameLen), local = buf.readUInt32LE(at + 42);
    at += 46 + nameLen + buf.readUInt16LE(at + 30) + buf.readUInt16LE(at + 32);
    if (name.endsWith('/')) continue;
    const start = local + 30 + buf.readUInt16LE(local + 26) + buf.readUInt16LE(local + 28), raw = buf.subarray(start, start + size);
    files.set(name, method === 8 ? inflateRawSync(raw) : raw);
  }
  const top = [...files.keys()].find((n) => n.endsWith('theme.json'))?.slice(0, -'theme.json'.length) || '';
  return new Map([...files].filter(([n]) => n.startsWith(top)).map(([n, b]) => [n.slice(top.length), b]));
}
async function themeWrite(req, res, path, q) {
  const parts = [];
  for await (const p of req) parts.push(p);
  const body = Buffer.concat(parts);
  let m;
  if (path === '/api/themes' && req.method === 'POST') {
    const files = unzip(body), manifest = files && files.get('theme.json') && JSON.parse(files.get('theme.json'));
    if (!manifest) return send(res, 400, { error: 'bad_request', message: 'the package has no theme.json' });
    if (existsSync(join(THEMES, manifest.id)) && q.get('replace') !== 'true') {
      return send(res, 409, { error: 'conflict', message: `a theme called ${manifest.name} (${manifest.id}) is already here` });
    }
    return send(res, 200, writeTheme(manifest.id, files));
  }
  if ((m = /^\/api\/themes\/([a-z0-9-]+)\/(.+)$/.exec(path)) && req.method === 'PUT') {
    if (!existsSync(join(THEMES, m[1])) && m[2] !== 'theme.json') return send(res, 404, { error: 'not_found', message: `no theme ${m[1]}; save its theme.json first` });
    const files = new Map([[m[2], body]]);
    if (!existsSync(join(THEMES, m[1], 'theme.css'))) files.set('theme.css', Buffer.alloc(0));
    return send(res, 200, writeTheme(m[1], files));
  }
  if ((m = /^\/api\/theme-pictures\/([a-z0-9-]+)\/([0-9a-f]{16})-(day|night)\.webp$/.exec(path)) && req.method === 'PUT') {
    const item = themeItem(m[1]);
    if (!item) return send(res, 404, { error: 'not_found', message: `no theme ${m[1]}` });
    if (!item.modes.includes(m[3])) return send(res, 400, { error: 'bad_request', message: `${item.name} has no ${m[3]} mode` });
    if (body.toString('latin1', 0, 4) !== 'RIFF' || body.toString('latin1', 8, 12) !== 'WEBP') return send(res, 400, { error: 'bad_request', message: 'a theme\'s picture is a WebP' });
    if (item.rev !== m[2]) return send(res, 409, { error: 'conflict', message: `${item.name} changed while its picture was drawn; draw it again` });
    for (const key of themePictures.keys()) if (key.startsWith(m[1] + '/') && !key.startsWith(`${m[1]}/${m[2]}-`)) themePictures.delete(key);
    themePictures.set(`${m[1]}/${m[2]}-${m[3]}`, body);
    return send(res, 204);
  }
  return send(res, 404, { error: 'not_found', message: `The fixture hub has no ${req.method} ${path}.` });
}

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

async function files(req, res, path, root = WEB) {
  const rel = normalize(decodeURIComponent(path === '/' ? '/index.html' : path)).replace(/^([/\\])+/, '');
  if (rel.startsWith('..')) return send(res, 404, { error: 'not_found', message: 'No such file.' });
  const f = join(root, rel);
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
  if (path === '/__fixture/emit' && req.method === 'POST') { const b = await readBody(req); emit(b.topic, b.data, req.headers['x-fixture-client']); return send(res, 204); }
  if (path === '/__fixture/likes-log' && req.method === 'GET') return send(res, 200, likeLog(clientOf(req)));
  // songs neither saved nor liked, with their entities: what a like may save
  if (path === '/__fixture/unsaved-songs' && req.method === 'GET') {
    const out = L.trackIds.filter((id) => !L.tracks[id].saved && !state.likes.includes(id)).slice(0, +(q.get('n') || 3));
    return send(res, 200, { items: out, entities: entities({ tracks: out, albums: albumsOf(out) }) });
  }
  if (path === '/__fixture/feed-shape' && req.method === 'POST') { feedShapes.set(clientOf(req), (await readBody(req)).shape); return send(res, 204); }
  if (path === '/__fixture/drop-events' && req.method === 'POST') {
    for (const [stream, c] of clients) if (c === clientOf(req)) { clients.delete(stream); stream.end(); }
    return send(res, 204);
  }
  if (path === '/__fixture/artist-late' && req.method === 'POST') { const b = await readBody(req); artistLate.set(clientOf(req) + ' ' + b.id, b); return send(res, 204); }
  if (path === '/__fixture/artist-in' && req.method === 'POST') { artistLate.delete(clientOf(req) + ' ' + (await readBody(req)).id); return send(res, 204); }
  if (path === '/api/events') {
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-store', connection: 'keep-alive' });
    res.write(': open\n\n');
    clients.set(res, clientOf(req)); req.on('close', () => clients.delete(res));
    return;
  }
  let m;
  const client = clientOf(req);
  if (path === '/__fixture/art-requests') { const counts = Object.fromEntries(artRequests.get(client) || []); artRequests.delete(client); return send(res, 200, counts); }
  if (path === '/__fixture/art-pending' && req.method === 'POST') { (await readBody(req)).ids.forEach((id) => artPending.add(client + ' ' + id)); return send(res, 204); }
  if (path === '/__fixture/art-cold' && req.method === 'POST') { goCold(client, (await readBody(req)).pace); return send(res, 204); }
  if (path === '/__fixture/screen') return send(res, 200, (screens.get(client) || { raw: null }).raw);
  if (path === '/__fixture/art-filled') return send(res, 200, (colds.get(client) || { filled: [] }).filled);
  if (path === '/api/pictures/screen' && req.method === 'PUT') { const out = report(client, await readBody(req)); return send(res, out.status, out.body); }
  if (path === '/__fixture/art-ready' && req.method === 'POST') {
    const { ids } = await readBody(req); ids.forEach((id) => artPending.delete(client + ' ' + id)); emit('art.ready', { keys: ids }, client); return send(res, 204);
  }
  if ((m = /^\/api\/(?:catalog\/)?art\/([^/]+)$/.exec(path))) {
    const seen = artRequests.get(client) || artRequests.set(client, new Map()).get(client);
    seen.set(url.pathname + url.search, (seen.get(url.pathname + url.search) || 0) + 1);
    // api.md /api/art: the sizes the hub serves; the fixture's pictures are 512 px at most, as a source with no more
    // is: never blown up, and X-Image-Size says so
    const asked = [96, 256, 512, 1024, 2048].includes(+q.get('size')) ? +q.get('size') : 256, size = Math.min(asked, 512);
    if (!L.albums[m[1]] && !L.artists[m[1]]) return send(res, 404, { error: 'not_found', message: 'No cover.' });
    if (artPending.has(client + ' ' + m[1]) || cold(client, m[1])) {
      res.writeHead(404, { 'content-type': 'application/json', 'cache-control': 'no-store' });
      return res.end(JSON.stringify({ error: 'not_found', message: 'The picture is still on its way.' }));
    }
    const png = cover(m[1], size);
    res.writeHead(200, { 'content-type': 'image/png', 'content-length': png.length, 'cache-control': 'private, max-age=86400', 'x-image-size': size + 'x' + size });
    return res.end(png);
  }
  if (path === '/__fixture/reads') { const tr = trickles.get(q.get('id')); return send(res, 200, tr ? { size: tr.size, reads: tr.reads } : null); }
  if (path === '/__fixture/trickle' && req.method === 'POST') { const b = await readBody(req); const out = trickle(b.id, b.bytesPerS, b.seekAhead, b.file); return send(res, out.status, out.body); }
  if ((m = /^\/api\/stream\/([^/]+)$/.exec(path)) && req.method === 'GET' && trickles.has(m[1])) return trickleAudio(req, res, m[1]);
  if ((m = /^\/api\/(library\/)?stream\/([^/]+)$/.exec(path)) && req.method === 'GET') return audio(req, res, m[2], !!m[1]);
  if ((m = /^\/api\/stream\/([^/]+)\/output-error$/.exec(path)) && req.method === 'POST') return outputError(res, m[1], (await readBody(req)).jobId);
  if ((m = /^\/api\/stream\/([^/]+)$/.exec(path)) && req.method === 'DELETE') return send(res, 204);
  if (THEMES && (m = /^\/api\/themes\/([a-z0-9-]+)\/theme\.json$/.exec(path)) && req.method === 'GET') {
    // the hub's manifest, its slots the templates in the folder (crates/themes store.rs)
    const item = themeItem(m[1]);
    if (!item) return send(res, 404, { error: 'not_found', message: `no theme ${m[1]}` });
    const { builtIn, rev, pictures, ...manifest } = item;
    return send(res, 200, manifest);
  }
  if (THEMES && (m = /^\/api\/themes\/([a-z0-9-]+)\/(.+)$/.exec(path)) && req.method === 'GET') return files(req, res, '/' + m[2], join(THEMES, m[1]));
  if (THEMES && (m = /^\/api\/theme-pictures\/([a-z0-9-]+)\/([0-9a-f]{16}-(?:day|night))\.webp$/.exec(path)) && req.method === 'GET') {
    const webp = themePictures.get(`${m[1]}/${m[2]}`);
    if (!webp) return send(res, 404, { error: 'not_found', message: `no picture ${m[2]} of theme ${m[1]}` });
    res.writeHead(200, { 'content-type': 'image/webp', 'content-length': webp.length, 'cache-control': 'private, max-age=31536000, immutable' });
    return res.end(webp);
  }
  // the listener's sound and surprise packs (ADR-0081): none here; lull's own are the static web/packs
  if (path === '/api/packs' && req.method === 'GET') return send(res, 200, { packs: [] });
  if (THEMES && (path === '/api/themes' || /^\/api\/(themes|theme-pictures)\/[a-z0-9-]+\/.+$/.test(path)) && ['POST', 'PUT'].includes(req.method)) return themeWrite(req, res, path, q);
  if (path === '/api/skins/mine' && req.method === 'POST') {
    const parts = [];
    for await (const p of req) parts.push(p);
    const out = addSkin(q.get('name') || '', Buffer.concat(parts));
    return send(res, out.status, out.body);
  }
  if ((m = /^\/api\/skins\/([0-9a-f]{32})\.wsz$/.exec(path)) && req.method === 'GET' && (ownSkins.has(m[1]) || SKINS)) {
    const file = ownSkins.has(m[1]) ? null : readSkins().byMd5.get(m[1]);
    if (!file && !ownSkins.has(m[1])) return send(res, 404, { error: 'not_found', message: 'The fixture hub has no skin ' + m[1] + '.' });
    const body = file ? readFileSync(file) : ownSkins.get(m[1]).bytes;
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': body.length, 'cache-control': 'public, max-age=31536000, immutable' });
    return res.end(body);
  }
  if (path.startsWith('/api/') || path === '/health') {
    const body = req.method === 'GET' || req.method === 'HEAD' ? undefined : await readBody(req);
    for (const [method, re, fn] of routes) {
      if (method !== req.method) continue;
      const mm = re.exec(path);
      if (!mm) continue;
      const out = fn(mm, q, body, client);
      if (out && out.status && (out.body !== undefined || out.status === 204)) return send(res, out.status, out.body);
      return send(res, 200, out);
    }
    return send(res, 404, { error: 'not_found', message: `The fixture hub has no ${req.method} ${path}.` });
  }
  return files(req, res, path);
});
server.listen(PORT, '127.0.0.1', () => console.log(`fixture hub on http://127.0.0.1:${PORT}`));
