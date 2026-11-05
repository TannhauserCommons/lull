// The fixture hub's library: invented artists, albums and songs, the same on every run (a seeded
// generator, no dates from the clock), big enough that list costs show: 1,600+ songs, a 1,200-song
// playlist, 300 likes, a week of history. Shapes follow docs/context/api.md.

const NOW = Date.UTC(2026, 10, 1, 12) / 1000; // 2026-11-01 12:00 UTC, the matrix's fixed clock
// every song this long instead (a long listening session in minutes: the memory soak)
const SONG_S = +(process.env.FIXTURE_TRACK_SECONDS || 0);
// a bigger library and a longer "Long drive" (how a screen's cost grows with a list's length)
const ALBUMS = Math.max(150, +(process.env.FIXTURE_ALBUMS || 0));
const LONG = +(process.env.FIXTURE_LONG_PLAYLIST || 1200);

// mulberry32: small, fast, deterministic
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const r = rng(20261101);
const pick = (xs) => xs[Math.floor(r() * xs.length)];
const int = (lo, hi) => lo + Math.floor(r() * (hi - lo + 1));
const hex = (s) => {
  let h = 2166136261;
  for (const c of s) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  let h2 = 5381;
  for (const c of s) h2 = (Math.imul(h2, 33) ^ c.charCodeAt(0)) >>> 0;
  return h.toString(16).padStart(8, '0') + h2.toString(16).padStart(8, '0');
};
const cap = (w) => w[0].toUpperCase() + w.slice(1);

const A = ['velvet', 'paper', 'quiet', 'amber', 'hollow', 'silver', 'lunar', 'tidal', 'cobalt', 'gentle', 'static', 'winter', 'golden', 'distant', 'neon', 'soft', 'iron', 'pale', 'wild', 'slow'];
const N = ['harbour', 'lanterns', 'orchard', 'signal', 'meadow', 'echoes', 'satellite', 'river', 'atlas', 'garden', 'parade', 'circuit', 'comet', 'island', 'mirror', 'season', 'window', 'forest', 'engine', 'choir'];
const V = ['falling', 'drifting', 'waiting', 'running', 'turning', 'glowing', 'sleeping', 'calling', 'fading', 'rising'];
const TAGS = ['dream pop', 'shoegaze', 'synthpop', 'ambient', 'indie rock', 'chillwave', 'post-punk', 'folk', 'house', 'jazz', 'downtempo', 'electronic'];
const FORMATS = [
  { format: 'FLAC 16/44.1', codec: 'FLAC', lossless: true, khz: 44.1, bits: 16, kbps: 880 },
  { format: 'FLAC 24/96', codec: 'FLAC', lossless: true, khz: 96, bits: 24, kbps: 2800 },
  { format: 'MP3 320', codec: 'MP3', lossless: false, khz: 44.1, bits: null, kbps: 320 },
  { format: 'Opus 160', codec: 'Opus', lossless: false, khz: 48, bits: null, kbps: 160 },
];

export const artists = {};
export const albums = {};
export const tracks = {};
export const songs = {}; // trackId -> { addedAt, plays }

for (let i = 0; i < 64; i++) {
  const name = i % 3 === 0 ? `The ${cap(pick(A))} ${cap(pick(N))}` : `${cap(pick(A))} ${cap(pick(N))}`;
  const id = 'ar-' + hex('artist:' + name + i);
  artists[id] = {
    id, name, tags: [pick(TAGS), pick(TAGS)].filter((t, k, xs) => xs.indexOf(t) === k),
    listeners: int(2_000, 900_000), bio: `${name} make ${pick(TAGS)} records from a small studio by the sea.`,
    musicbrainzId: null,
  };
}
const artistIds = Object.keys(artists);

let added = NOW - 400 * 86400;
for (let i = 0; i < ALBUMS; i++) {
  const artistId = artistIds[i % artistIds.length];
  const artist = artists[artistId].name;
  const title = i % 4 === 0 ? `${cap(pick(N))} ${cap(pick(N))}` : `${cap(pick(V))} ${cap(pick(A))} ${cap(pick(N))}`;
  const id = 'al-' + hex('album:' + title + i);
  const year = 1994 + (i * 7) % 32;
  const f = FORMATS[i % FORMATS.length];
  // the last albums are the catalog's, not the library's: Soulseek songs, unsaved
  const saved = i < 140 || i >= 150;
  const n = int(8, 14);
  const tagsOf = artists[artistId].tags;
  const trackIds = [];
  for (let k = 1; k <= n; k++) {
    const t = k % 5 === 0 ? `${cap(pick(V))}` : `${cap(pick(A))} ${cap(pick(N))}${k % 7 === 0 ? ' (Reprise)' : ''}`;
    const tid = 't-' + hex('track:' + id + k);
    const duration = SONG_S || int(140, 420);
    tracks[tid] = {
      id: tid, title: t, artistId, artist, albumId: id, album: title, year, no: k, disc: 1, duration, tags: tagsOf,
      saved, ...f, sizeBytes: Math.round((duration * f.kbps * 1000) / 8),
    };
    added += int(600, 30_000);
    if (saved) songs[tid] = { addedAt: new Date(Math.min(added, NOW - 3600) * 1000).toISOString(), plays: int(0, 60) };
    trackIds.push(tid);
  }
  albums[id] = {
    id, title, artistId, artist, year, type: i % 9 === 0 ? 'EP' : 'album', released: String(year), label: null,
    tags: tagsOf, saved: saved ? 'all' : 'none', trackIds, duration: trackIds.reduce((s, t) => s + tracks[t].duration, 0),
    musicbrainzId: null,
  };
}
export const trackIds = Object.keys(tracks);
// the library's songs and albums; the rest are the catalog's
export const savedIds = trackIds.filter((id) => tracks[id].saved);
export const savedAlbumIds = Object.keys(albums).filter((id) => albums[id].saved === 'all');
// streaming this album fails: no peer has it (the player's failed state)
export const failingAlbum = Object.keys(albums)[149];
// this album's songs arrive, but as bytes no browser can decode (a copy that can't be played here)
export const brokenAlbum = Object.keys(albums)[148];
export const albumIds = Object.keys(albums);
export { artistIds };

export const tags = TAGS;
export const likes = trackIds.filter((_, i) => i % 5 === 0).slice(0, 300);

export const playlists = [
  { name: 'Long drive', desc: 'Everything for the coast road', ids: trackIds.slice(0, LONG) },
  { name: 'Morning', desc: '', ids: trackIds.filter((_, i) => i % 11 === 0).slice(0, 60) },
  { name: 'Focus', desc: 'Quiet ones', ids: trackIds.filter((_, i) => i % 17 === 3).slice(0, 40) },
  { name: 'Late night', desc: '', ids: trackIds.filter((_, i) => i % 23 === 5).slice(0, 25) },
].map((p, i) => ({
  id: 'pl-' + (i + 1), name: p.name, desc: p.desc, created: NOW - (40 - i) * 86400, updated: NOW - i * 3600, cover: null,
  items: p.ids.map((trackId, k) => ({ trackId, added: NOW - (40 - i) * 86400 + k * 60 })),
}));

// A week of plays, newest first, split into UTC days (the specs run with tz=0).
export const history = (() => {
  const days = [];
  let at = NOW - 1800;
  for (let d = 0; d < 7; d++) {
    const items = [];
    for (let k = 0; k < 30; k++) {
      const trackId = trackIds[(d * 53 + k * 7) % trackIds.length];
      const duration = tracks[trackId].duration;
      const completed = k % 4 !== 0;
      items.push({
        playId: String(10_000 - d * 100 - k), trackId, at, playedS: completed ? duration : Math.round(duration / 3),
        completed, skipped: !completed, scrobble: completed ? 'done' : 'short', sendable: false,
      });
      at -= int(240, 1200);
    }
    days.push({ day: new Date(items[0].at * 1000).toISOString().slice(0, 10), items });
  }
  return days;
})();

export const lyrics = (trackId) => {
  const t = tracks[trackId];
  if (!t) return null;
  const lines = [];
  for (let s = 8, k = 0; s < t.duration - 5; s += 6, k++) lines.push({ t: s, text: `${cap(V[k % V.length])} through the ${N[(k * 3) % N.length]}, ${A[(k * 7) % A.length]} and ${A[(k * 5 + 1) % A.length]}` });
  return { kind: 'synced', lines, source: 'LRCLIB' };
};

export { NOW };
