// A lull link (docs/adr/0034): /1/<kind>/<names>[/cover|/card]?<ids>. Everything a page or a cover needs is read
// from the address, checked here, and trusted after: names are exact, ids keep their form or are dropped.

/** The names each kind carries in its path, and the ids (query keys) it may carry, in the order links write them. */
const KINDS = {
  song: { names: ['artist', 'title'], keys: ['al', 'aa', 'len', 'it', 'ic', 'dz', 'mb'] },
  album: { names: ['artist', 'title'], keys: ['it', 'dz', 'mb'] },
  artist: { names: ['artist'], keys: ['dz', 'mb'] },
};
const VERSION = '1';
const MAX_NAME = 200;
const DIGITS = /^[0-9]{1,15}$/;
/** What may follow a link's names: its cover (an image) or its Discord card (JSON). */
const PARTS = ['cover', 'card'];
const MBID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
/** How each query key's value is checked: a malformed id is dropped rather than trusted. */
const CHECK = {
  al: name, aa: name,
  len: (v) => (DIGITS.test(v) && +v > 0 && +v <= 86400 ? v : null),
  it: id, ic: id, dz: id,
  mb: (v) => (MBID.test(v.toLowerCase()) ? v.toLowerCase() : null),
};

function id(v) { return DIGITS.test(v) ? v : null; }
function name(v) {
  const s = v.normalize('NFC').trim();
  return s && s.length <= MAX_NAME && !/[\u0000-\u001f\u007f]/.test(s) ? s : null;
}

/**
 * What an address names: `{ kind, names: {artist, title?}, ids: {…}, part }`, or null for anything that isn't a
 * lull link. `part` is `cover` or `card` for those addresses (`…/cover`, `…/card`), else null: the page.
 */
export function parse(url) {
  const parts = url.pathname.split('/').slice(1);
  if (parts[0] !== VERSION) return null;
  const kind = KINDS[parts[1]];
  if (!kind) return null;
  const rest = parts.slice(2);
  const part = rest.length === kind.names.length + 1 && PARTS.includes(rest[rest.length - 1]) ? rest[rest.length - 1] : null;
  if (rest.length !== kind.names.length + (part ? 1 : 0)) return null;
  const names = {};
  for (let i = 0; i < kind.names.length; i++) {
    let decoded;
    try { decoded = decodeURIComponent(rest[i]); } catch { return null; }
    const n = name(decoded);
    if (!n) return null;
    names[kind.names[i]] = n;
  }
  const ids = {};
  for (const key of kind.keys) {
    const v = url.searchParams.get(key);
    const ok = v === null ? null : CHECK[key](v);
    if (ok) ids[key] = ok;
  }
  return { kind: parts[1], names, ids, part };
}

/** The item's page, path and query, in the one order links are written. */
export function pathOf(item) {
  const kind = KINDS[item.kind];
  const path = ['', VERSION, item.kind, ...kind.names.map((n) => encodeURIComponent(item.names[n]))].join('/');
  const query = kind.keys.filter((k) => item.ids[k]).map((k) => k + '=' + encodeURIComponent(item.ids[k])).join('&');
  return { path, query: query ? '?' + query : '' };
}

export function pageUrl(origin, item) {
  const { path, query } = pathOf(item);
  return origin + path + query;
}

export function coverUrl(origin, item) {
  return partUrl(origin, item, 'cover');
}

export function cardUrl(origin, item) {
  return partUrl(origin, item, 'card');
}

function partUrl(origin, item, part) {
  const { path, query } = pathOf(item);
  return origin + path + '/' + part + query;
}

/** The public page for it: song.link or album.link by its iTunes id, else Last.fm's by name (ADR-0032's three hosts). */
export function publicPage(item) {
  const { kind, names, ids } = item;
  if (kind === 'song' && ids.it) return 'https://song.link/i/' + ids.it;
  if (kind === 'album' && ids.it) return 'https://album.link/i/' + ids.it;
  const segments = [names.artist];
  if (kind === 'song') segments.push('_', names.title);
  if (kind === 'album') segments.push(names.title);
  return 'https://www.last.fm/music/' + segments.map(encodeURIComponent).join('/');
}

/** The album (or artist) whose picture it shows: what a cover is looked up for and kept under. */
export function subjectOf(item) {
  const { kind, names, ids } = item;
  if (kind === 'artist') return { kind: 'artist', artist: names.artist, dz: ids.dz, mb: ids.mb };
  if (kind === 'album') return { kind: 'album', artist: names.artist, album: names.title, it: ids.it, dz: ids.dz, mb: ids.mb };
  return {
    kind: ids.al ? 'album' : 'song',
    artist: ids.aa || names.artist,
    album: ids.al,
    song: names.title,
    songArtist: names.artist,
    it: ids.ic, track: ids.it, dz: ids.dz, mb: ids.mb,
  };
}

/** One key per album or artist, so every song of an album shares one lookup: its best id, else its names. */
export function subjectKey(s) {
  if (s.kind === 'artist') return 'artist:' + (s.mb ? 'mb:' + s.mb : s.dz ? 'dz:' + s.dz : 'name:' + loose(s.artist));
  if (s.it) return 'album:it:' + s.it;
  if (s.dz) return 'album:dz:' + s.dz;
  if (s.mb) return 'album:mb:' + s.mb;
  if (s.kind === 'album') return 'album:name:' + loose(s.artist) + '\u0000' + loose(s.album);
  return 'song:' + (s.track ? 'it:' + s.track : 'name:' + loose(s.songArtist) + '\u0000' + loose(s.song));
}

/** A name with case, accents and punctuation set aside, for comparing names from two services. */
export function loose(s) {
  return s.normalize('NFKD').replace(/\p{M}/gu, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
}
