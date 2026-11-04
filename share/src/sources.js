// Where a cover comes from (docs/adr/0034, "The cover never breaks"): every source lull tracks, ids first (exact),
// names after. Each step answers a picture's address that was fetched and is an image, or null when that source
// has none; a source that couldn't answer (its turn not given, a timeout, a 429 or 5xx) throws Busy, so the
// lookup is tried again soon instead of being kept as "none".

import { loose } from './link.js';

export const USER_AGENT = 'lull-share/1 (+https://github.com/TannhauserCommons/lull)';
const STEP_MS = 3000;
const MAX_BYTES = 2 * 1024 * 1024;
const SIZE = 600;

/** A source that couldn't answer now; `pause` when it asked us to slow down (429, 403). */
export class Busy extends Error {
  constructor(service, pause = false) { super(service + ' is busy'); this.service = service; this.pause = pause; }
}

/** The steps for an album (or a song's album) and for an artist, in order. */
const ALBUM = [itunesLookup, deezerAlbum, coverArtArchive, lastfm, itunesSearch];
const ARTIST = [deezerArtist, wikidata, deezerArtistSearch];

/**
 * The first picture any source has for `subject`: `{ url, source }`, `{ none: true }` when every source answered
 * that it has none, or `{ busy: true }` when one couldn't answer and none had one.
 * `gate.take(service)` gives a turn of that service's budget; `gate.pause(service)` stops it for a while.
 */
export async function find(subject, { gate, lastfmKey }) {
  let busy = false;
  for (const step of subject.kind === 'artist' ? ARTIST : ALBUM) {
    try {
      const url = await step(subject, { gate, lastfmKey });
      if (url) return { url, source: step.service };
    } catch (e) {
      if (!(e instanceof Busy)) throw e;
      busy = true;
      if (e.pause) await gate.pause(e.service);
    }
  }
  return busy ? { busy: true } : { none: true };
}

/** A picture answers 200 with an image no larger than 2 MB. Its body is left unread. */
async function isPicture(url, service) {
  const res = await get(url, service, { accept: 'image/*', api: false });
  if (!res) return false;
  await res.body?.cancel();
  return looksLikePicture(res);
}

/** A response that is a picture by its headers: a common image type, not empty, no larger than 2 MB. */
export function looksLikePicture(res) {
  const type = res.headers.get('content-type') || '';
  const length = Number(res.headers.get('content-length') ?? '1');
  return res.ok && /^image\/(jpeg|png|webp|gif)/i.test(type) && length > 0 && length <= MAX_BYTES;
}

/** One request to `service`: the response when it answered 200, null for a 404 (or a 400: an id it doesn't know).
    A 429 or 403 from a service's API pauses that service; one from a picture's CDN doesn't. */
async function get(url, service, { accept = 'application/json', api = true } = {}) {
  let res;
  try {
    res = await fetch(url, { headers: { 'user-agent': USER_AGENT, accept }, signal: AbortSignal.timeout(STEP_MS), redirect: 'follow' });
  } catch {
    throw new Busy(service);
  }
  if (res.ok) return res;
  await res.body?.cancel();
  if (res.status === 404 || res.status === 400 || res.status === 410) return null;
  throw new Busy(service, api && (res.status === 429 || res.status === 403));
}

async function json(url, service, gate) {
  if (!(await gate.take(service))) throw new Busy(service);
  const res = await get(url, service);
  if (!res) return null;
  try { return await res.json(); } catch { throw new Busy(service); }
}

/** Keeps `url` when it is a picture: `isPicture`, with the service the address came from. */
async function picture(url, service) {
  return url && (await isPicture(url, service)) ? url : null;
}

// ---- iTunes ----

/** The store's 100 px cover, at SIZE: its CDN scales to the size named in the address. */
function storeCover(artwork100) {
  return artwork100 && artwork100.includes('100x100bb') ? artwork100.replace('100x100bb', `${SIZE}x${SIZE}bb`) : null;
}

async function itunesLookup(s, { gate }) {
  const id = s.it || s.track;
  if (!id) return null;
  const body = await json(`https://itunes.apple.com/lookup?id=${id}`, 'itunes', gate);
  return picture(storeCover(body?.results?.[0]?.artworkUrl100), 'itunes');
}
itunesLookup.service = 'itunes';

/** By names, taking only a listing whose artist and title are the link's once case, accents and punctuation are set
    aside: a cover that might be another album's is worse than lull's card. */
async function itunesSearch(s, { gate }) {
  const album = s.kind === 'album';
  const title = album ? s.album : s.song;
  const artist = album ? s.artist : s.songArtist;
  if (!title) return null;
  const q = new URLSearchParams({ term: `${artist} ${title}`, entity: album ? 'album' : 'song', limit: '10' });
  const body = await json(`https://itunes.apple.com/search?${q}`, 'itunes', gate);
  const same = (body?.results || []).find((r) =>
    loose(r.artistName || '') === loose(artist) && loose((album ? r.collectionName : r.trackName) || '') === loose(title));
  return picture(storeCover(same?.artworkUrl100), 'itunes');
}
itunesSearch.service = 'itunes';

// ---- Deezer ----

/** Deezer answers its errors with a 200: code 4 is its quota, anything else means it has no such thing. */
function deezerBody(body) {
  if (body?.error) {
    if (body.error.code === 4) throw new Busy('deezer', true);
    return null;
  }
  return body;
}

/** A Deezer picture at SIZE; its CDN scales to the size named in the address. An empty md5 is its placeholder. */
function deezerPicture(url) {
  if (!url || /\/images\/(cover|artist)\/\//.test(url)) return null;
  return url.replace(/\/\d+x\d+-/, `/${SIZE}x${SIZE}-`);
}

async function deezerAlbum(s, { gate }) {
  if (!s.dz) return null;
  const body = deezerBody(await json(`https://api.deezer.com/album/${s.dz}`, 'deezer', gate));
  return picture(deezerPicture(body?.cover_xl), 'deezer');
}
deezerAlbum.service = 'deezer';

async function deezerArtist(s, { gate }) {
  if (!s.dz) return null;
  const body = deezerBody(await json(`https://api.deezer.com/artist/${s.dz}`, 'deezer', gate));
  return picture(deezerPicture(body?.picture_xl), 'deezer');
}
deezerArtist.service = 'deezer';

/** Only when exactly one Deezer artist has that name. */
async function deezerArtistSearch(s, { gate }) {
  const q = new URLSearchParams({ q: s.artist, limit: '10' });
  const body = deezerBody(await json(`https://api.deezer.com/search/artist?${q}`, 'deezer', gate));
  const same = (body?.data || []).filter((a) => loose(a.name || '') === loose(s.artist));
  return same.length === 1 ? picture(deezerPicture(same[0].picture_xl), 'deezer') : null;
}
deezerArtistSearch.service = 'deezer';

// ---- MusicBrainz's Cover Art Archive ----

async function coverArtArchive(s, { gate }) {
  if (!s.mb) return null;
  if (!(await gate.take('musicbrainz'))) throw new Busy('musicbrainz');
  return picture(`https://coverartarchive.org/release-group/${s.mb}/front-500`, 'musicbrainz');
}
coverArtArchive.service = 'coverartarchive';

// ---- Last.fm ----

/** Last.fm's star placeholder, which it answers for anything it has no picture of. */
const LASTFM_PLACEHOLDER = '2a96cbd8b46e442fc41c2b86b821562f';

async function lastfm(s, { gate, lastfmKey }) {
  if (!lastfmKey) return null;
  const album = s.kind === 'album';
  if (!album && !s.song) return null;
  const q = new URLSearchParams(album
    ? { method: 'album.getinfo', artist: s.artist, album: s.album }
    : { method: 'track.getinfo', artist: s.songArtist, track: s.song });
  q.set('api_key', lastfmKey);
  q.set('format', 'json');
  const body = await json(`https://ws.audioscrobbler.com/2.0/?${q}`, 'lastfm', gate);
  if (body?.error === 29) throw new Busy('lastfm', true);
  const images = (album ? body?.album?.image : body?.track?.album?.image) || [];
  const largest = images.map((i) => i['#text']).filter(Boolean).pop();
  return largest && !largest.includes(LASTFM_PLACEHOLDER) ? picture(largest, 'lastfm') : null;
}
lastfm.service = 'lastfm';

// ---- Wikidata ----

async function wikidata(s, { gate }) {
  if (!s.mb) return null;
  const sparql = `SELECT ?image WHERE { ?artist wdt:P434 "${s.mb}"; wdt:P18 ?image } LIMIT 1`;
  const body = await json(`https://query.wikidata.org/sparql?format=json&query=${encodeURIComponent(sparql)}`, 'wikidata', gate);
  const file = body?.results?.bindings?.[0]?.image?.value;
  if (!file) return null;
  // P18 is Commons' Special:FilePath address of the file; it scales to `width`.
  const url = new URL(file.replace(/^http:/, 'https:'));
  url.searchParams.set('width', String(SIZE));
  return picture(url.href, 'wikidata');
}
wikidata.service = 'wikidata';
