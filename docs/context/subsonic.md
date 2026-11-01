# Subsonic API

OpenSubsonic at `/rest/*`, so existing apps (Feishin, Symfonium, DSub, play:Sub…) browse, stream and scrobble against the hub. Code: `crates/subsonic`. Endpoint list: [api.md](api.md#subsonic-cratessubsonic).

## How it fits

- **Over the seams only.** Browsing, audio and covers come from `Library` (`artists`, `albums`, `album`, `saved`, `search`, `stream`, `cover`); likes, playlists and plays go through `Collection`, so the web UI, history, taste and Last.fm see them as if the web player had made them.
- **Reads two core tables directly:** `likes` (when each song was liked: Subsonic's `starred`) and `plays` joined to `catalog_tracks` (album lists `frequent` and `recent`). No tables of its own.
- **Ids are lull's ids** (`ar-`, `al-`, `t-`, `pl-`). An album id is also the cover art id of the album and its songs; an artist's `coverArt` is one of its albums.
- **Only saved songs.** A client can only stream what the library has, so playlists and stars list their saved songs. `updatePlaylist`'s `songIndexToRemove` counts those and is mapped back to positions in the whole playlist.
- **One music folder** (id 1); `getIndexes` is `getArtists` again, since a lull library has no folder tree.

## Auth (ADR-0002)

- The hub's gate leaves `/rest/*` to `auth.rs`. Every endpoint but `getOpenSubsonicExtensions` needs one of:
  - `u=lull` plus `p=` (plain or `enc:` hex) or `t=md5(password + salt)` with `s=`, where the password is an app password from `POST /api/devices/subsonic` (`devices::subsonic`);
  - `apiKey=`: any paired device's token (`devices::verify`), an app password included.
- **Errors** follow the spec: 10 missing parameter, 40 wrong credentials, 43 `apiKey` with `u`/`p`/`t`/`s` or `p` with `t`, 44 bad API key, 70 not found, 0 anything else. Like every Subsonic server, failures are HTTP 200 with `status="failed"`.

## Responses

- `f=json` gives JSON; anything else (default) gives XML. JSONP is not offered.
- Bodies are built once as JSON and turned into XML by the usual convention: scalars are attributes, objects child elements, arrays repeated elements, a `value` key is the element's text. Unknown facts are left out, never `null`.
- The envelope carries `type: "lull"`, `serverVersion`, `openSubsonic: true`, API version `1.16.1`.
- Parameters come from the query and, for `application/x-www-form-urlencoded` POSTs, the body (OpenSubsonic `formPost`). Extensions advertised: `apiKeyAuthentication`, `formPost`.

## Spec gaps (deliberate, until lull has the facts)

- **No transcoding:** `stream` and `download` both send the saved file through `Library::stream` (Range honoured, 200 or 206); `maxBitRate`, `format` and `timeOffset` are ignored.
- **Songs only can be starred:** `star`/`unstar` with `albumId` or `artistId` fail with code 0; `getStarred2` has no albums or artists; album lists `starred` and `highest` (no ratings) are empty.
- **No "added" date:** album list `newest` orders by release date then year; `AlbumID3.created` is left out.
- **Now playing:** `scrobble?submission=false` is accepted without effect (no seam yet). A submission is recorded as a completed play of the song's length starting at `time`, or one song length ago.
- **Genres are album tags;** `getRandomSongs` filters by the song's album (genre, year).
- **Not built:** `getLyricsBySongId` (waits on a lyrics seam), bookmarks, play queue, shares, podcasts, internet radio, users, chat, folder browsing (`getMusicDirectory`), the non-ID3 `getAlbumList`/`getStarred`/`search2`.
- App passwords signing in by token don't update the device's `lastSeen`; `apiKey` sign-ins do.

## Clients tested

None live yet: the crate is unit-tested through its router against fakes of both seams (token auth, JSON/XML envelope, range streaming, playlist index mapping, scrobble). Record each client tried against the homelab here, with what didn't work.
