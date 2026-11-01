# Subsonic API

OpenSubsonic at `/rest/*`, so existing apps (Feishin, Symfonium, DSub, play:Sub…) browse, stream and scrobble against the hub. Code: `crates/subsonic`. Endpoint list: [api.md](api.md#subsonic-cratessubsonic).

## How it fits

- **Over the seams only.** Browsing, audio and covers come from `Library` (`artists`, `albums`, `album`, `saved`, `search`, `stream`, `cover`); likes, playlists and plays go through `Collection`, so the web UI, history, taste and Last.fm see them as if the web player had made them; lyrics come from `Lyrics`, the same answer as `/api/lyrics`.
- **Reads two core tables directly:** `likes` (when each song was liked: Subsonic's `starred`) and `plays` joined to `catalog_tracks` (album lists `frequent` and `recent`). No tables of its own.
- **Ids are lull's ids** (`ar-`, `al-`, `t-`, `pl-`). An album id is also the cover art id of the album and its songs; an artist's `coverArt` is one of its albums.
- **Only saved songs.** A client can only stream what the library has, so playlists and stars list their saved songs. `updatePlaylist`'s `songIndexToRemove` counts those and is mapped back to positions in the whole playlist.
- **One music folder** (id 1); `getIndexes` is `getArtists` again, since a lull library has no folder tree.

## Auth (ADR-0002)

- The hub's gate leaves `/rest/*` to `auth.rs`. Every endpoint but `getOpenSubsonicExtensions` needs one of:
  - `u=lull` plus `p=` (plain or `enc:` hex) or `t=md5(password + salt)` with `s=`, where the password is an app password from `POST /api/devices/subsonic` (`devices::subsonic`);
  - `apiKey=`: any paired device's token (`devices::verify`), an app password included.
- **`getUser`** answers for `lull` only (70 for any other name), with the roles lull can honour: stream, download, playlists, scrobbling. Clients like Feishin sign in through it.
- **Errors** follow the spec: 10 missing parameter, 40 wrong credentials, 43 `apiKey` with `u`/`p`/`t`/`s` or `p` with `t`, 44 bad API key, 70 not found, 0 anything else. Like every Subsonic server, failures are HTTP 200 with `status="failed"`.

## Responses

- `f=json` gives JSON; anything else (default) gives XML. JSONP is not offered.
- Bodies are built once as JSON and turned into XML by the usual convention: scalars are attributes, objects child elements, arrays repeated elements, a `value` key is the element's text. Unknown facts are left out, never `null`.
- The envelope carries `type: "lull"`, `serverVersion`, `openSubsonic: true`, API version `1.16.1`.
- Parameters come from the query and, for `application/x-www-form-urlencoded` POSTs, the body (OpenSubsonic `formPost`). Extensions advertised: `apiKeyAuthentication`, `formPost`, `songLyrics`.

## Spec gaps (deliberate, until lull has the facts)

- **No transcoding:** `stream` and `download` both send the saved file through `Library::stream` (Range honoured, 200 or 206); `maxBitRate`, `format` and `timeOffset` are ignored.
- **Songs only can be starred:** `star`/`unstar` with `albumId` or `artistId` fail with code 0; `getStarred2` has no albums or artists; album lists `starred` and `highest` (no ratings) are empty.
- **No "added" date:** album list `newest` orders by release date then year; `AlbumID3.created` is left out.
- **Now playing:** `scrobble?submission=false` is accepted without effect (no seam yet). A submission is recorded as a completed play of the song's length starting at `time`, or one song length ago.
- **Genres are album tags;** `getRandomSongs` and `getSongsByGenre` go by the song's album (genre, year).
- **Lyrics:** one `structuredLyrics` entry, `lang: und` (LRCLIB doesn't say); synced lines carry `start` in ms, plain lyrics are untimed lines, instrumental songs and misses an empty list. The legacy `getLyrics` (by artist and title) isn't offered.
- **Not built:** bookmarks, play queue, shares, podcasts, internet radio, user management, chat, scan status, artist and album info, top and similar songs, ratings, folder browsing (`getMusicDirectory`), the non-ID3 `getAlbumList`/`search2`. `getStarred` is `getStarred2` under the old key.
- App passwords signing in by token don't update the device's `lastSeen`; `apiKey` sign-ins do.

## Clients tested

The crate is unit-tested through its router against fakes of the seams (token auth, `getUser`, JSON/XML envelope, range streaming, playlist index mapping, scrobble, lyrics). Record each client tried against the homelab here, with what didn't work.

- **curl, 2026-11-01** (homelab at 00d3452, a two-album library): every endpoint then built (all above but `getUser`, `getStarred`, `getSongsByGenre`) answered as documented. Covered: token, plain, `enc:` and `apiKey` sign-ins with errors 10/40/43/44/70; browse and every album list type; `search3` with and without a query; `stream`/`download` Range (206, right `Content-Range` at both ends of a FLAC); covers by album and song id; star/unstar round trip; a playlist created, renamed, edited by index, replaced and deleted; `scrobble?submission=false`; synced lyrics. A submission (`submission=true`) wasn't sent, so as not to put a test play in the listener's history and Last.fm.
- **Feishin's calls, 2026-11-01** (homelab at 359cbb9): its sign-in, server info and main screens replayed from its source (`development` branch) with its exact parameters (`c=Feishin`, `v=1.13.0`, token auth, `.view` paths, empty-query `search3` paging, hand-built `stream` and `getCoverArt` URLs). The app itself wasn't run: an agent may not install it or type a password into it. All passed: `getUser`, `ping`, extensions, music folders, the home album lists, albums, album, artists, artist, songs, search, random, star, `getStarred`, unstar, genres, `getSongsByGenre`, now playing, playlists, a ranged stream (206) and a cover. Expected misses, each one section Feishin shows as empty or failed, never a broken sign-in: `jukeboxControl` (Feishin turns jukebox off), `getArtistInfo2` (no biography), `getTopSongs` and `getSimilarSongs2` (an artist page's top songs and artist radio). Feishin reads `contentType` without a null check, so a song whose codec library doesn't know would break a list. Lyrics for one song were unavailable because LRCLIB itself answered 503 for it; `/api/lyrics` answers the same.
