// The hub's answers as the UI reads them (docs/context/api.md): one definition, held by two hubs.
// - The real hub (run.mjs: the image in CI's `ui` job, the Linux package in `package-contract`) must send every key
//   named here, of its type, and may send more.
// - The fixture hub the UI gates run against (tests/web/fixture-hub-contract.test.mjs, in `mise run check`) must send
//   them too, and nothing else: a field the fixture serves that isn't named here is one no hub was checked to serve,
//   so the gates could pass against an API lull no longer has. Name it here (run.mjs then checks the hub for it) or
//   take it out of the fixture.
// Types: 'string' 'number' 'boolean' 'any'; a trailing '?' allows null. [shape] is a list of that shape, tuple(...) a
// fixed list, dict(shape) an object of any keys (ids, section names) whose values have that shape, orNull(shape) that
// shape or null, opt(shape) a key the hub leaves out when it has nothing to say (serde's skip_serializing_if). A list or
// dict of objects that comes back empty checks nothing inside it: `check` names it, so a run says what it didn't see.

const OPT = Symbol('opt');
const OR_NULL = Symbol('orNull');
const DICT = Symbol('dict');
const TUPLE = Symbol('tuple');
export const opt = (shape) => ({ [OPT]: shape });
export const dict = (shape) => ({ [DICT]: shape });
export const tuple = (...shapes) => ({ [TUPLE]: shapes });
export const orNull = (shape) => ({ [OR_NULL]: shape });

const typeOf = (v) => (v === null ? 'null' : Array.isArray(v) ? 'list' : typeof v);

/**
 * The ways `value` departs from `shape`, each as `<path>: <what>`, and the lists and dicts left empty (their insides
 * unchecked). `closed`: a key the shape doesn't name is a departure (the fixture); else it is allowed (the hub).
 */
export function check(value, shape, { closed }) {
  const problems = [], empty = [];
  (function walk(v, s, at) {
    if (typeof s === 'string') {
      const nullable = s.endsWith('?'), type = nullable ? s.slice(0, -1) : s;
      if (type === 'any' || (v === null && nullable)) return;
      if (typeOf(v) !== type) problems.push(`${at || '.'}: ${typeOf(v)}, not ${s}`);
      return;
    }
    if (s[OR_NULL]) { if (v !== null) walk(v, s[OR_NULL], at); return; }
    if (Array.isArray(s)) {
      if (!Array.isArray(v)) return problems.push(`${at || '.'}: ${typeOf(v)}, not a list`);
      if (!v.length && typeof s[0] !== 'string') empty.push(at || '.');
      v.forEach((e, i) => walk(e, s[0], `${at}[${i}]`));
      return;
    }
    if (s[TUPLE]) {
      if (!Array.isArray(v) || v.length !== s[TUPLE].length) return problems.push(`${at || '.'}: not a list of ${s[TUPLE].length}`);
      s[TUPLE].forEach((t, i) => walk(v[i], t, `${at}[${i}]`));
      return;
    }
    if (typeOf(v) !== 'object') return problems.push(`${at || '.'}: ${typeOf(v)}, not an object`);
    if (s[DICT]) {
      const keys = Object.keys(v);
      if (!keys.length && typeof s[DICT] !== 'string') empty.push(at || '.');
      keys.forEach((k) => walk(v[k], s[DICT], `${at}.${k}`));
      return;
    }
    for (const [k, inner] of Object.entries(s)) {
      if (inner[OPT] !== undefined) { if (k in v) walk(v[k], inner[OPT], `${at}.${k}`); }
      else if (!(k in v)) problems.push(`${at}.${k}: missing`);
      else walk(v[k], inner, `${at}.${k}`);
    }
    if (closed) for (const k of Object.keys(v)) if (!(k in s)) problems.push(`${at}.${k}: not in the contract`);
  })(value, shape, '');
  return { problems, empty };
}

// ---------- the entities (lull_core::model) ----------
const ids = ['string'];
export const Track = {
  id: 'string', title: 'string', artistId: 'string', artist: 'string', albumId: 'string', album: 'string',
  year: 'number?', no: 'number?', disc: 'number?', duration: 'number?', tags: ['string'], saved: 'boolean',
  // the file's facts, flattened in: none for a song lull has no file of
  format: opt('string'), codec: opt('string'), lossless: opt('boolean'), khz: opt('number?'), bits: opt('number?'),
  kbps: opt('number?'), sizeBytes: opt('number?'),
};
export const Album = {
  id: 'string', title: 'string', artistId: 'string', artist: 'string', year: 'number?', type: 'string?', released: 'string?',
  label: 'string?', tags: ['string'], saved: 'string', trackIds: ids, duration: 'number?', musicbrainzId: 'string?',
};
export const Artist = { id: 'string', name: 'string', tags: ['string'], listeners: 'number?', bio: 'string?', musicbrainzId: 'string?' };
// each kind left out when it holds none
const entities = { tracks: opt(dict(Track)), albums: opt(dict(Album)), artists: opt(dict(Artist)) };

const source = (type) => ({ source: 'string', value: type });
const minimum = { formats: ['string'], kbps: 'number', losslessOnly: 'boolean' };
const service = { state: 'string', detail: 'string', extra: opt('any') };
const build = { release: 'string', changes: 'number', commit: 'string?', install: 'string' };

/**
 * Every read the contract checks: the path (given the ids `needs` names, taken from the hub's own library) and the
 * shape of its answer. Only reads a hub answers without reaching a service outside it on the asking (no Soulseek
 * search, lyrics, skin museum or radio), so both hubs answer them the same on every run.
 */
export const READS = [
  { path: () => '/health', open: true, shape: {
    version: 'string', build, flags: ['string'],
    services: { soulseek: service, library: service, lastfm: service, cache: service, catalog: service, youtube: service },
  } },
  { path: () => '/api/setup', open: true, shape: {
    claimed: 'boolean', soulseekManaged: 'boolean', steps: { library: 'boolean', soulseek: 'boolean', lastfm: 'boolean' },
  } },
  { path: () => '/api/updates', shape: {
    version: 'string', build, state: 'string', latest: orNull({ version: 'string', url: 'string', publishedAt: 'string?' }),
    checkedAt: 'number?', error: 'string?',
  } },
  { path: () => '/api/devices', shape: {
    me: 'string',
    items: [{ id: 'string', name: 'string', kind: 'string', createdAt: 'number', lastSeen: 'number', private: 'boolean', online: 'boolean' }],
  } },
  { path: () => '/api/config', shape: {
    items: [{ key: 'string', value: 'string?', set: 'boolean', source: 'string', secret: 'boolean', restart: 'boolean' }],
  } },
  { path: () => '/api/settings', shape: {
    playback: {
      gapless: 'boolean', crossfade: 'number', normalisation: 'string', loudnessTarget: 'number', volumeStep: 'number',
      radioWhenQueueEnds: 'boolean', prefetchCount: 'number',
    },
    sources: {
      maxPeerQueue: 'number', stallTimeout: 'number',
      presets: [{ id: 'string', name: 'string', stream: { below: 'string', maxKbps: 'number', minimum, pick: 'string' } }],
      devices: dict('string'),
      save: { autoUpgrade: 'boolean', below: 'string', minimum, pick: 'string', upgradeEveryH: 'number' },
      youtube: { mode: 'string', format: 'string', match: 'string', waitS: 'number', saves: 'boolean', autoUpdate: 'boolean' },
      preferredFormats: source(['string']),
    },
    search: { showFiles: 'boolean' },
    library: { duplicatePolicy: 'string', autoSaveLikes: 'boolean', musicFolder: source('string?') },
    sharing: { uploadSlots: 'number', uploadLimitKBs: 'number', mode: source('string') },
    discovery: { adventurousness: 'number', on: { listenbrainz: 'boolean', musicbrainz: 'boolean', lastfm: 'boolean' } },
    cache: { capGB: source('number'), usedGB: 'number' },
    app: { version: 'string', server: source('string?') },
  } },
  { path: () => '/api/sources/presets', shape: {
    presets: [{ id: 'string', name: 'string', builtin: 'boolean', stream: { below: 'string', maxKbps: 'number', minimum, pick: 'string' } }],
    device: { id: 'string', preset: 'string', chosen: 'boolean', phone: 'boolean' },
  } },
  { path: () => '/api/themes', shape: {
    items: [{
      id: 'string', name: 'string', author: 'string', api: 'number', extends: 'string?', shell: 'string', swatches: ['string'],
      modes: ['string'], description: 'string', preview: 'boolean', slots: ['string'], coverVariety: 'boolean',
      builtIn: 'boolean', pack: 'boolean',
      // a listener's theme: its files' rev, the pictures made of it, the live values it names
      rev: opt('string'), pictures: opt(['string']), live: opt(['string']), window: opt('any'),
    }],
  } },
  { path: () => '/api/packs', shape: { packs: ['any'] } },
  { path: () => '/api/lastfm', shape: {
    configured: 'boolean', connected: 'boolean', user: 'string?', scrobbling: 'boolean', useHistory: 'boolean', queued: 'number',
  } },
  { path: () => '/api/youtube', shape: {
    state: 'string', detail: 'string', mode: 'string', backoffUntil: 'number?',
    ytdlp: { version: 'string?', updated: 'boolean', pinnedVersion: 'string?', latest: 'string?', checkedAt: 'number?', error: 'string?' },
  } },
  { path: () => '/api/library/source', shape: {
    mode: 'string', musicDir: 'string?', jellyfin: 'any', files: 'number?', lastScan: 'number?', switching: 'boolean',
  } },
  { path: () => '/api/jobs', shape: { items: ['any'] } },
  { path: () => '/api/transfers', shape: { items: ['any'], total: 'number' } },
  { path: () => '/api/import/links', shape: { links: ['any'] } },
  { path: () => '/api/bookmarks', shape: { items: ['any'] } },
  { path: () => '/api/skins/favourites', shape: { items: ['any'] } },
  { path: () => '/api/skins/mine', shape: {
    hub: 'string', folder: 'string?', watching: 'boolean', problem: 'string?', items: ['any'], skipped: ['any'],
  } },
  { path: () => '/api/likes', shape: { items: ids, total: 'number', entities } },
  { path: () => '/api/likes/unsaved', shape: { items: ids, total: 'number' } },
  { path: () => '/api/follows', shape: { artists: ids, tags: ['string'], entities, unsure: opt(ids) } },
  { path: () => '/api/playlists', shape: {
    items: [{
      id: 'string', name: 'string', desc: 'string', created: 'number', updated: 'number', cover: 'string?',
      sort: { key: 'string', dir: 'string' }, items: [{ trackId: 'string', added: 'number' }],
    }],
    total: 'number', entities,
  } },
  { path: () => '/api/history', shape: {
    days: [{ day: 'string', items: [{
      playId: 'string', trackId: 'string', at: 'number', playedS: 'number', completed: 'boolean', skipped: 'boolean',
      scrobble: 'string?', sendable: 'boolean',
    }] }],
    next: 'string?', entities,
  } },
  { path: () => '/api/stats', shape: {
    minutes: 'number', plays: 'number', artists: 'number', saves: 'number', skipRate: 'number', newArtists: 'number',
    hitRate: { recommended: 'number', liked: 'number', saved: 'number' },
    topArtists: [tuple('string', 'number')], topTracks: [tuple('string', 'number')], topTags: [tuple('string', 'number')],
    clock: ['number'], entities,
  } },
  { path: () => '/api/taste', shape: { artists: [tuple('string', 'number')], tags: [tuple('string', 'number')], entities } },
  { path: () => '/api/library/songs', shape: {
    items: ids, total: 'number', entities, songs: dict({ addedAt: 'string', plays: 'number' }), stale: 'boolean',
  } },
  { path: () => '/api/library/albums', shape: { items: ids, total: 'number', entities, stale: 'boolean' } },
  { path: () => '/api/library/artists', shape: { items: ids, total: 'number', entities, stale: 'boolean' } },
  { path: () => '/api/library/genres', shape: {
    items: [{ id: 'string', name: 'string', artists: 'number', songs: 'number', trackIds: ids }], total: 'number', entities, stale: 'boolean',
  } },
  { path: () => '/api/feed', shape: {
    updated: 'number', summary: 'string', newReleases: ids,
    picks: [{
      trackId: 'string', via: 'string', avail: opt(['string']), noSource: opt('boolean'),
      reasons: [{ kind: 'string', name: 'string', ref: opt('string'), evidence: opt('any') }],
    }],
    radioStarters: [{ id: 'string', title: 'string', sub: 'string', seed: { kind: 'string', id: 'string' } }],
    jumpBackIn: [{ kind: 'string', id: 'string' }],
    followedTags: ['string'], coldStartTags: ['string'], cold: 'boolean', coldStartArtists: opt(ids),
    errors: opt(dict('string')),
    // each followed tag with the tags that come with it, once the hub knows some
    entities: { ...entities, tags: opt([{ id: 'string', name: 'string', related: ['string'] }]) },
  } },
  { needs: 'track', path: (x) => `/api/tracks/${x.track}`, shape: { track: Track, entities } },
  { needs: 'album', path: (x) => `/api/albums/${x.album}`, shape: { album: Album, partial: opt('boolean'), entities } },
  { needs: 'artist', path: (x) => `/api/artists/${x.artist}`, shape: {
    artist: { ...Artist, image: opt('any'), about: opt('string?') },
    topTracks: ids, counts: opt(dict({ plays: 'number', listeners: 'number' })), popularVia: opt('string'), albums: ids,
    discography: { albums: ids, singles: ids, compilations: ids, appearsOn: ids }, similar: ids,
    pending: opt(['string']), errors: opt(dict('string')), entities,
  } },
  { needs: 'tag', path: (x) => `/api/tags/${encodeURIComponent(x.tag)}`, shape: {
    tag: { id: 'string', name: 'string', desc: 'string?', related: ['string'] }, artists: ids, newReleases: ids,
    errors: opt(dict('string')), entities,
  } },
];

/**
 * The ids the reads that need one ask about, from the hub's own library: its first song, that song's album and the
 * album's artist, and the song's first tag. Missing when the library is empty.
 */
export async function libraryIds(get) {
  const songs = (await get('/api/library/songs?limit=1')).json;
  const track = songs?.items?.[0];
  const t = track && songs.entities?.tracks?.[track];
  return { track, album: t?.albumId, artist: t?.artistId, tag: t?.tags?.[0] };
}
