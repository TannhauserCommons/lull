# Discovery: sources, limits and how the feed is built

Owner: Discovery session (`crates/discovery`). Source choice and why: [ADR-0030](../adr/0030-discovery-sources.md). Taste model: [taste.md](taste.md).

## Sources and their limits

| Source | Used for | Limit lull keeps | Cached | Auth |
|---|---|---|---|---|
| ListenBrainz `api.listenbrainz.org/1` | fresh releases, CF recommendations, tag radio, artist radio, recording metadata | 1 call/s (server allows 30 per 10 s per IP; 429 beyond) | fresh releases 6 h; radio and recommendations 1 day; recording metadata 30 days | none; `LULL_LISTENBRAINZ_TOKEN` sent when set; `LULL_LISTENBRAINZ_USER` enables recommendations |
| ListenBrainz labs `labs.api.listenbrainz.org` | similar artists | same pace | 7 days | none |
| MusicBrainz `musicbrainz.org/ws/2` | finding artists and release groups by name, discographies, tracklists, tags, tag + date search | 1.1 s between calls (1/s per IP average; going over gets every call a 503). Other lull crates share the IP | lookups 30 days; discographies 7 days; tag searches 12 h | none; User-Agent `lull/<ver> ( repo URL )` from `lull_core::http` |
| Last.fm `ws.audioscrobbler.com/2.0` | artist bio and listeners, similar artists (artist pages), tag description, tag top artists, similar tags; `track.search` and `artist.search` (catalog search) | 0.5 s between calls (no published number; "several per second" risks suspension) | 7 days (tag text 30); searches 1 day | `LULL_LASTFM_API_KEY` or the built-in key; off without one |
| Deezer `api.deezer.com` | catalog search (tracks, albums, artists, with covers, photos and popularity; only artists where its catalog is closed); artist photos for artist pages; `/infos` (is it open here?) at start and daily | 50 calls per 5 s, kept by a sliding window in `sources/deezer.rs`. A quota error arrives as HTTP 200 with `error.code` 4: the window closes for 5 s and the call is tried once more | searches 10 min | none |
| iTunes Search `itunes.apple.com/search` | catalog search where Deezer is closed: one `entity=song` call per query (25), albums derived from the songs, store `LULL_CATALOG_COUNTRY` (default US) | about 20 calls a minute (Apple's guidance); lull spends at most 18 per rolling minute, then searches without it | searches 10 min | none |
| Cover Art Archive `coverartarchive.org` | covers of albums MusicBrainz found (`front-250/500/1200`) | none published; one fetch per album and size, cached | pictures 30 days | none |

- **Pacing** goes through `Http::pace`, shared with every lull crate. MusicBrainz uses core's `MUSICBRAINZ_PACE`, because library calls it from the same IP.
- **Caching:** rows unused for 90 days are pruned at start-up. When a source fails, the expired answer is served (`sources/cache.rs`).
- **Switches (Settings > Discovery):** collection's `settings` event carries `discovery.on.{listenbrainz,musicbrainz,lastfm}`. A source switched off is not used at all, cached answers included. Its calls fail with "switched off in Settings > Discovery", which shows up in `errors`, and the feed rebuilds.
- **Adventurousness:** `discovery.adventurousness` from the same event is radio's `steer` when the UI sends none (0.4 until then).
- Tokens that ListenBrainz gates (`/explore/lb-radio`, `/popularity/*`) aren't needed: top tracks come from artist radio with no similar artists.
- MusicBrainz tag + date search uses Lucene ranges: `tag:"x" AND firstreleasedate:[2026-08-02 TO *]`.

## The feed (`GET /api/feed`)

Built in the background; the first build happens while the first caller waits (tens of seconds, paced). Then:
- kept in memory and in `discovery_state` (served after a restart);
- rebuilt when six hours old, when follows change, or after feedback or a taste reset;
- hidden artists are filtered at serve time.

| Shelf | From |
|---|---|
| `newReleases` | ListenBrainz fresh releases (30 days, every size). Ranked: followed artists, then liked artists (taste), then releases tagged with followed or liked tags (albums and EPs above singles). 12. |
| `picks` | Four pools, interleaved round-robin. One track per artist. Nothing already played or liked, nothing by hidden or disliked artists. Except fresh releases, no artist the listener already likes. 12. |
| `radioStarters` | Top artist, followed tags, the strongest liked tag not followed, the track played 3+ times this week; on a cold start, three cold-start tags |
| `jumpBackIn` | Albums played (not skipped) in the last 30 days, most recent first (5), then the month's most played artist |
| `followedTags` | `follows` at serve time |
| `coldStartTags` | A fixed list of niche tags across genres (`feed.rs`) |

The four pick pools:

| Pool | Reason | via |
|---|---|---|
| Artist radio (medium, 0-60% popularity) from the top 4 liked artists, minus the seed | `like` (seed artist) | `ListenBrainz similar artists` |
| Tag radio at 0-50% popularity for followed tags, then the top liked *specific* tags (4 tags; 3 rotating cold-start tags when nothing is known) | `tag` | `ListenBrainz tags` |
| CF recommendations (only with `LULL_LISTENBRAINZ_USER`) | `listeners` ("listeners like you", no `ref`) | `ListenBrainz recommendations` |
| First track of up to 3 fresh releases by followed or liked artists | `fresh` | `MusicBrainz releases` |

`errors` names a shelf only when it came back empty because a source failed. Partial failures are logged.

## Radio (`GET /api/radio`)

- **Seed `tag`:** ListenBrainz tag radio.
- **Seed `artist`:** artist radio (15 similar artists, 4 recordings each).
- **Seed `track`:** its artist's radio plus its first specific tag's radio.
- **Finding the artist** (`radio.rs`): a credit is tried whole, then without its featured part, then artist by artist (split on "&", "," and "x"), then the featured artists ("Shawn Chrystopher feat. Cults & Ashly" → Shawn Chrystopher). Each name goes through what lull remembers, then MusicBrainz (an unavailable or bad answer retried once; after that MusicBrainz isn't asked for the other names), then Last.fm's `artist.getInfo` MBID.
- **Without an MBID**, or when ListenBrainz has no artist radio for it, the radio comes from the artist radio of Last.fm's most similar artist with an MBID, plus tag radio for the first specific tag of the seed or the artist's Last.fm tags. Only when all of that is empty does the call fail: with MusicBrainz's or ListenBrainz's error if one failed, else 404.

Umbrella genres (rock, pop, electronic, hip hop, indie…: `taste::UMBRELLA_TAGS`) are never dug into: they lead to charts, not niches. They stay in the taste profile.
- **`steer` (0 familiar … 1 adventurous, in quarter steps so answers are shared):**
  - picks the ListenBrainz mode: easy < 0.34 ≤ medium < 0.67 ≤ hard;
  - sets the popularity window: 0 is 40-100%, 1 is 0-50%;
  - nudges liked artists up (familiar) or down (adventurous).
- **Each call returns 12 tracks:** at most 2 per artist (except the seed artist), minus `exclude`, hidden and disliked artists.

## Pages

| Page | From |
|---|---|
| **Album** | MusicBrainz release group (by remembered MBID, else search) and the earliest official release's tracklist. Without a match, the catalog's tracks of that album |
| **Artist** | MusicBrainz tags and discography; Last.fm bio and listeners; top tracks from artist radio, sorted by listens; similar from Last.fm (else ListenBrainz) |
| **Tag** | MusicBrainz releases tagged it in the last 90 days; Last.fm description, top artists and similar tags. Without Last.fm: artists from tag radio, and related tags from tag co-occurrence on the releases |

A section whose source failed is empty and named in `errors`.

## Search (`GET /api/search`, ADR-0008)

Code: `src/search/` (`mod.rs` runs a query, `merge.rs` merges and ranks), `src/art.rs` (pictures).

- **The fast catalog is chosen per hub** (`search/catalog.rs`): Deezer's `/infos` is asked at start and daily (hourly after a failure).
  - Where Deezer is open (`open: true`, and until it has answered), Deezer answers tracks, albums and artists.
  - Where it is closed (as from the homelab: track and album searches come back empty with `total` above zero), iTunes Search answers tracks and albums, and Deezer only artists and photos.
  - The choice is the `catalog` status (`/health`, the status tray): "catalog: Deezer", or "catalog: iTunes (US) + Deezer artists, as Deezer isn't open in this country". It turns `warn` with the reason when a fast source fails.
- **Sources, side by side:**
  - Deezer: tracks (25), albums (15), artists (10), or only artists where it is closed;
  - iTunes, where Deezer is closed: songs (25), with the albums they are on (`Creep - EP` is the EP "Creep"). Apple gives no counts, so its order stands in for popularity. Over its budget it answers nothing, and search goes on with the rest;
  - saved songs through the `Library` seam (25);
  - MusicBrainz: **one** recording search per query, `(recording:(w) AND artist:(w)) OR recording:"w" OR artist:"w"` on the normalized words. Its release groups and credited artists become album and artist rows. Rows that don't hold the typed words are dropped (the query is broad);
  - Last.fm `track.search` and `artist.search`, when a key is configured and the switch is on.

  - for a **submitted** query (`submitted=1`), MusicBrainz release groups by title too, paced after the recording search: Bandcamp-only EPs and albums are found by their name ("skronk of the times" → HAIRY BABIES' EP). Rows that don't hold the words are dropped.

  MusicBrainz and Last.fm are asked only for 3+ characters: MusicBrainz's one call a second is shared with every lull crate.
- **Timing:** the answer goes out once the fast sources (Deezer, iTunes, the library) are in, or at 400 ms if it has rows by then. With none by 400 ms it waits for every fast source, up to 1.5 s, so the answer has a top result and ranked rows. From the homelab iTunes takes 0.6-1 s and Deezer's artist search about 0.6 s, measured 2026-11-01. The rest arrives as `search.more` with only rows not shown yet, plus entities of shown rows that gained facts (art, length). The log line `search answered` has the milliseconds.
- **Runs:** one per query (lowercase, single spaces) for 10 minutes, shared by every request in that time, so a repeat answers at once. A run where a source failed isn't reused.
- **Merging** (`merge.rs`): one song per shared ISRC, or per `ids::loose_artist` + `ids::loose_title` (feat., remaster, version, edit and live tags dropped; the same normalization Soulseek matches files with) with lengths within 3 s or unknown. A different ISRC doesn't keep rows apart: remasters and compilations get new ones. Albums merge by loose artist and title, artists by loose name.
- **A row keeps** the identity (and so the id) of its first source, and gains the others' facts. Before the first answer, a saved copy lends its id, and a row without a release takes the first named one.
- **Ranking:** rows another source confirms come before Last.fm-only ones (scrobbles carry misspellings: "Radiohed - Creep"); then fit (exact title or name, an artist's songs and albums for their exact name, or artist and title together; else every typed word starts a word of the row; else loose), then popularity in tenths, then the listener's own (saved, liked, a followed artist), then exact popularity. Popularity is a log scale per source: Deezer rank (1e6 = 1), Deezer fans (1e7), Last.fm listeners (tracks 5e6, artists 1e7). Deezer albums have no count, so their order stands in. MusicBrainz rows have none.
- **Misspelt artists:** a Last.fm-only artist one or two edits from a confirmed artist (at most one per four letters), with under 1% of its fans or listeners, isn't listed ("Radiohed" next to Radiohead).
- **Top result:** the best of the first artist, album and track: confirmed before Last.fm-only, then fit, then popularity, with artists leaning +0.15 and albums +0.05 ("radiohed" is Radiohead, "creep" is the song).
- **`search.catalog`** names the 50 best tracks once every source is in (at the answer when all were in by then).
**Deezer withholds its catalog where it isn't open.** From the homelab's IP, `/search/track` and `/search/album` answer `data: []` with `total` above zero, and `/infos` says `open: false`. Only `/search/artist` and lookups by id work. Should Deezer be searched for tracks there anyway (before `/infos` has answered), that is named in `errors.deezer` ("isn't open in this country") rather than taken as "nothing found".

## Pictures (`GET /api/catalog/art/{id}?size=96|256|512`)

- **Sources:** Deezer covers and artist photos (any size, from `md5_image` / `picture_xl`), iTunes covers (any size, from `artworkUrl100`), and Cover Art Archive covers for MusicBrainz albums (the next archive size up). Last.fm is never a picture source.
- **Where each picture comes from** is `lull_core::catalog` (`catalog_albums.art`, `catalog_artists.image`, a URL with `{size}`). Search records it, and the artist page looks the photo up on Deezer by exact name once.
- **Entities:** albums carry `art` and artists `image`, `api/catalog/art/<id>` relative to the page (the UI adds `?size=`), only when a source is known. Albums without one use library's `/api/art/{albumId}`.
- **Cache:** `<data dir>/catalog-art/<id>-<size>`, 30 days; "the source has none" for a day.

## Ids

Album and artist ids are hashes (`lull_core::ids`), so discovery remembers every album and artist it shows (`discovery_albums`, `discovery_artists`). An unknown id falls back to the catalog: the first track with that `albumId` / `artistId`. Tracks go to the catalog. An id the catalog already has keeps every fact it has, because another crate may have added file facts. Discovery only fills its gaps: year, number, disc, length and tags.
