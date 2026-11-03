# Discovery: sources, limits and how the feed is built

Owner: Discovery session (`crates/discovery`). Source choice and why: [ADR-0030](../adr/0030-discovery-sources.md). Taste model: [taste.md](taste.md).

## Sources and their limits

| Source | Used for | Limit lull keeps | Cached | Auth |
|---|---|---|---|---|
| ListenBrainz `api.listenbrainz.org/1` | fresh releases, CF recommendations, tag radio, artist radio, recording metadata | 1 call/s (server allows 30 per 10 s per IP; 429 beyond) | fresh releases 6 h; radio and recommendations 1 day; recording metadata 30 days | none; `LULL_LISTENBRAINZ_TOKEN` sent when set; `LULL_LISTENBRAINZ_USER` enables recommendations |
| ListenBrainz labs `labs.api.listenbrainz.org` | similar artists | same pace | 7 days | none |
| MusicBrainz `musicbrainz.org/ws/2` | finding artists and release groups by name, discographies (with credits), tracklists, tags, an artist's links to its iTunes and Deezer pages (`inc=url-rels`, with its tags in the same call), tag + date search | 1.1 s between calls (1/s per IP average; going over gets every call a 503). Other lull crates share the IP | lookups and artist links 30 days (a release group not found: 1 day, as new releases reach MusicBrainz after the catalogs); discographies 7 days; tag searches 12 h | none; User-Agent `lull/<ver> ( repo URL )` from `lull_core::http` |
| Last.fm `ws.audioscrobbler.com/2.0` | artist bio and listeners, similar artists and top tracks with play counts (artist pages), tag description, tag top artists, similar tags; `track.search` and `artist.search` (catalog search) | 0.5 s between calls (no published number; "several per second" risks suspension) | 7 days (tag text 30); searches and top tracks 1 day | `LULL_LASTFM_API_KEY` or the built-in key; off without one |
| Deezer `api.deezer.com` | catalog search (tracks, albums, artists, with covers, photos and popularity; only artists where its catalog is closed); artists by name and id, and covers from an artist's radio (both answer where it is closed), for artist photos; an artist's albums and top tracks where it is open; `album/{id}/tracks` for the album page of an album it found (lookups by id answer where it is closed too); `/infos` (is it open here?) at start and daily | 50 calls per 5 s, kept by lull's one Deezer window (`lull_core::http::DEEZER`, `Http::window`), which collection's playlist links share. A quota error arrives as HTTP 200 with `error.code` 4: the window closes for 5 s and the call is tried once more | searches 10 min; tracklists and artists 7 days; an artist's albums and top 1 day | none |
| iTunes Search `itunes.apple.com/search` | catalog search where Deezer is closed: per query an `entity=song` call (25; their albums come along) and an `entity=album` call (25) side by side, store `LULL_CATALOG_COUNTRY` (default US); `lookup?id=<collectionId>&entity=song` for the album page of an album it found; `entity=song` or `entity=album` (10) for [share links](#share-links); for artist pages `entity=musicArtist` (10) and the artist's `lookup?id=<artistId>&entity=album` and `entity=song` (200 each) | about 20 calls a minute (Apple's guidance); lull spends at most 18 per rolling minute, then searches without it. Album searches only while 6 of them stay for songs | searches 10 min; tracklists 7 days; artists by name 30 days; an artist's lookups 1 day | none |
| Wikidata `query.wikidata.org/sparql` | artist photos on Wikimedia Commons (P18) and iTunes and Deezer ids, by MBID (P434), every artist of a page in one query | 1 query/s (the service limits processing time per minute) | 30 days per MBID | none; lull's User-Agent |
| Cover Art Archive `coverartarchive.org` | covers of albums MusicBrainz found (`front-250/500/1200`) | none published; one fetch per album and size, cached | pictures 30 days | none |

- **Pacing** goes through `Http::pace`, shared with every lull crate. MusicBrainz uses core's `MUSICBRAINZ_PACE`, because library calls it from the same IP.
- **MusicBrainz, ListenBrainz, Last.fm and Wikidata are bounded** (`sources::Breaker`): each call times out after 5 s (`Http` retries a timeout, so about 17 s at most). After two failed calls in a row the service is left alone for a minute: calls fail at once with "isn't answering". A call abandoned before it answered (a radio or search out of time) counts as failed.
- **Search sources are capped at 8 s:** one that hasn't answered by then is named in `errors`, so every run ends with `search.more { done: true }`.
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
| `newReleases` | ListenBrainz fresh releases (30 days, every size). Ranked: followed artists, then liked artists (taste), then releases tagged with followed or liked tags (albums and EPs above singles). Only what can play: of the best 30, the 12 best that are saved or that Soulseek peers share (`feed::check_shelf`). Remembered answers (`Transfers::album_known`) fill the shelf at once; then `album_available` searches one release at a time, best first, and never for one ranked below a full shelf. The build waits 3 s for answers; later ones re-serve the shelf and announce `feed.updated`. A release without an answer is never shown, so the shelf is empty rather than wrong while Soulseek can't say; those are asked again every 5 minutes (free while Soulseek is offline or its checks wait). A stored feed loaded after a restart is checked again |
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
- **Without an MBID**, or when ListenBrainz has no artist radio for it, ListenBrainz is asked for the artist radio of Last.fm's most similar artist with an MBID, plus tag radio for the first specific tag of the seed or the artist's Last.fm tags.
- **When ListenBrainz gives nothing** (down, slower than 5 s, or empty), Last.fm fills the radio: `track.getSimilar` for a track seed; the artist's top tracks plus those of three similar artists (further down Last.fm's list as `steer` rises) for an artist seed or when a track has no similar tracks; `tag.getTopTracks` for a tag. These songs name no release.
- **Then the saved library:** songs matching the artist (without its featured part) or the tag.
- **8 s at most** for the whole call. It fails only when no source gave tracks: 503 with the reason when none answered at all, else 404.

Umbrella genres (rock, pop, electronic, hip hop, indie…: `taste::UMBRELLA_TAGS`) are never dug into: they lead to charts, not niches. They stay in the taste profile.
- **`steer` (0 familiar … 1 adventurous, in quarter steps so answers are shared):**
  - picks the ListenBrainz mode: easy < 0.34 ≤ medium < 0.67 ≤ hard;
  - sets the popularity window: 0 is 40-100%, 1 is 0-50%;
  - nudges liked artists up (familiar) or down (adventurous).
- **Each call returns 12 tracks:** at most 2 per artist (except the seed artist), minus `exclude`, hidden and disliked artists.
- **Originals first:** a title whose brackets or " - " suffix say remix, live, edit, mix, version or dub ("Talamak (Tom Lown Remix)") only fills a batch after every original, unless the seed track is such a take.

## Pages

| Page | From |
|---|---|
| **Album** | An album iTunes or Deezer found (search remembers its `collectionId` / album id in `discovery_album_sources`): that catalog's tracklist, one call (about 0.1-0.5 s from here); MusicBrainz then adds its MBID, tags, type and date in the background. Otherwise the MusicBrainz release group (by remembered MBID, else search by artist and title, version tags ignored) and the earliest official release's tracklist: two calls behind the shared 1.1 s pace. Without a match, the catalog's tracks of that album. **The page answers within 3 s:** a tracklist not in by then is still fetched and remembered, the page answers with the tracks lull has (`partial: true`), and `album.updated` follows |
| **Artist** | See [Artist pages](#artist-pages) |
| **Tag** | MusicBrainz releases tagged it in the last 90 days; Last.fm description (without its "Read more on Last.fm" tail), top artists (with their Deezer photos) and similar tags. Without Last.fm: artists from tag radio, and related tags from tag co-occurrence on the releases. 404 when no source knows the tag (and none failed) |

A section whose source failed is empty and named in `errors`. **Artist and tag pages answer within 4 s:** their sections run at once, each in a task of its own. On a tag page, one not in by then is named in `errors` and, cached when it ends, shows on the next visit; an artist page names it in `pending` and announces `artist.updated` once every late section has ended.

### Artist pages

Code: `src/artist/` (`mod.rs` the page, `discography.rs`, `popular.rs`, `photo.rs`). Why: [ADR-0031](../adr/0031-artist-pages.md).

- **Who it is:** the MBID a release named (feed, search, album and tag pages remember each album's artist with it), else Last.fm's by name, else MusicBrainz's. With the MBID known the sections start at once; without, they wait for those two (within the 4 s).
- **Namesakes are told apart** (BIJI shares the name with another Biji on iTunes and on Deezer):
  - MusicBrainz's links (`artist_links`) name the iTunes and Deezer artists. Without a link, iTunes's best-known artist of exactly that name (case and a leading "the" aside) is taken, and Deezer's only when it is the only one of that name.
  - A Last.fm profile asked by MBID that comes back by name with another MBID is another artist's: listeners, bio and tags are left out.
  - Popular songs are only songs a catalog lists for this artist (Last.fm pools every artist of a name).
  - A credit "leads" a release when the artist is its whole main part or its first artist ("BIJI & JUGGLERZ", not "Conducta & BIJI" or "CHAMOS feat. BIJI"); credited anywhere counts for songs. Similar artists that are credit strings ("Kalman Chodakiewitz, Eli Shiffman & Petite Meller" beside Petite Meller) are dropped.
- **Discography** (`discography.rs`): iTunes's artist lookup (`entity=album`, every release the store lists for the artist, its own when the collection's `artistId` is the artist's, plus the albums its songs are on that those miss: soundtracks, compilations, other artists' EPs; DJ mixes left out), Deezer's artist albums where it is open, and MusicBrainz's release groups (with credits). Merged by loose title (case, punctuation, "(Explicit)", "(Deluxe)" aside), within the artist's own releases or those it appears on, and within a year; iTunes, then Deezer, then MusicBrainz gives a release's identity (its tracklist and cover come from that catalog in one call; the explicit edition before the clean one), the others add the MBID, a more precise date, a cover. Grouped like Spotify: `albums`, `singles` (singles and EPs), `compilations`, `appearsOn` (any release the artist doesn't lead), newest first. Each is remembered with its catalog source, so its album page asks that catalog.
- **Popular** (`popular.rs`): Last.fm's `artist.getTopTracks` (30, with play and listener counts; takes of one song summed: "Boya" and "Boya (from Cyberpunk: Edgerunners 2)"), placed on the catalog's songs (Deezer's artist top where open, then the iTunes song lookup's order, which follows popularity), on the artist's own release when it has one (the single over the series soundtrack). The catalog's order fills the 10 rows; without Last.fm it is the order. With no catalog songs: Last.fm's titles alone, else ListenBrainz artist radio sorted by listens. `counts` carries Last.fm's numbers, `popularVia` the source.
- **Similar:** Last.fm by MBID (else by name), else ListenBrainz; 12.
- **Photos** (`photo.rs`, the artist and its similar artists, after the catalog): never an album cover posing as one. Deezer shows a cover for an artist without a photo (BIJI's "photo" was the Cyberpunk: Edgerunners 2 cover). So: Wikimedia Commons by way of Wikidata (P18 for the MBID); else Deezer's picture of the surely-same Deezer artist (linked by MusicBrainz or Wikidata, or the only one of that name), unless a 64-bit difference hash of a 96 px copy is within 10 bits of one of the artist's covers (the discography's newest 8, Deezer radio's). With no covers to compare with, no photo. The verdict is kept in `discovery_artist_photos` (a photo 30 days, "none" 7); changing it drops the cached files and changes the `?v=` of `image`. **Only a vetted photo is advertised or served**, anywhere: the picture search records for an artist may be a cover. An artist any response names before it is vetted (search, feed, tag and similar artists) goes to a background queue (64 deep, 12 to a batch, one Wikidata query each, an artist being vetted never twice at once; the artist page holds its own artist, which it vets against the discography's covers), so its photo shows from a later read. Without a photo, each design draws its own fallback.
- **Cost of a first visit:** MusicBrainz 2 calls (links, release groups; 3 without an MBID), iTunes 3 (artist search, two lookups, only from what search leaves spare), Deezer 1 (+2 where open, + per similar artist a name search and a radio), Last.fm 3 (profile, top tracks, similar), Wikidata 1, and small picture fetches to vet Deezer pictures. Later visits are cached (see the table).

## Search (`GET /api/search`, ADR-0008)

Code: `src/search/` (`mod.rs` runs a query, `merge.rs` merges and ranks), `src/art.rs` (pictures).

- **The fast catalog is chosen per hub** (`search/catalog.rs`): Deezer's `/infos` is asked at start and daily (hourly after a failure).
  - Where Deezer is open (`open: true`, and until it has answered), Deezer answers tracks, albums and artists.
  - Where it is closed (as from the homelab: track and album searches come back empty with `total` above zero), iTunes Search answers tracks and albums, and Deezer only artists and photos.
  - The choice is the `catalog` status (`/health`, the status tray): "catalog: Deezer", or "catalog: iTunes (US) + Deezer artists, as Deezer isn't open in this country". It turns `warn` with the reason when a fast source fails.
- **Sources, side by side:**
  - Deezer: tracks (25), albums (15), artists (10), or only artists where it is closed;
  - iTunes, where Deezer is closed: songs (25), with the albums they are on (`Creep - EP` is the EP "Creep"), and albums by their own name (25), so a soundtrack or compilation is found although none of its songs is named like the query. "Various Artists" albums are compilations. Apple gives no counts, so its order stands in for popularity (albums on Deezer's album scale, 0.6 at most; albums only known from a song at half its popularity). Over its budget it answers nothing, and search goes on with the rest; album search failing leaves the songs and shows in `errors.itunes`;
  - saved songs through the `Library` seam (25);
  - MusicBrainz: **one** recording search per query, `(recording:(w) AND artist:(w)) OR recording:"w" OR artist:"w"` on the normalized words. Its release groups and credited artists become album and artist rows, one artist row per credited artist ("Radiohead & Portishead" is two). Bootlegs, broadcasts, interviews and spoken word don't count as releases, and a recording only on those is left out ("Interview CD", radio sessions). A song's album is its first official album or soundtrack (else EP or single, else a compilation or live release). Songs and artists that don't hold every typed word are dropped (the query is broad); albums holding most of them stay;
  - Last.fm `track.search` and `artist.search`, when a key is configured and the switch is on.

  - for a **submitted** query (`submitted=1`), MusicBrainz release groups by title too, paced after the recording search: Bandcamp-only EPs and albums are found by their name ("skronk of the times" → HAIRY BABIES' EP). Rows that don't hold most of the words are dropped.

  MusicBrainz and Last.fm are asked only for 3+ characters: MusicBrainz's one call a second is shared with every lull crate.
- **Timing:** the answer goes out once every fast source (Deezer, iTunes, the library) is in, or at 1.5 s, so its top result and order come from all of them (an answer at 400 ms with only Deezer's artists in led "blinding lights" with a small artist of that name, and shown rows are never reordered). From the homelab iTunes takes 0.6-1 s and Deezer's artist search about 0.6 s, measured 2026-11-01. The rest arrives as `search.more` with only rows not shown yet, plus entities of shown rows that gained facts (art, length). The log line `search answered` has the milliseconds.
- **Runs:** one per query (lowercase, single spaces) for 10 minutes, shared by every request in that time, so a repeat answers at once. A run where a source failed isn't reused.
- **Merging** (`merge.rs`): one song per shared ISRC, or per `ids::loose_artist` + `ids::loose_title` (feat., remaster, version, edit and live tags dropped; the same normalization Soulseek matches files with) with lengths within 3 s or unknown. A different ISRC doesn't keep rows apart: remasters and compilations get new ones. Albums merge by loose artist and title, artists by loose name.
- **A row keeps** the identity (and so the id) of its first source, and gains the others' facts. Before the first answer, a saved copy lends its id, and a row without a release takes the first named one.
- **Ranking:** rows another source confirms come before Last.fm-only ones (scrobbles carry misspellings: "Radiohed - Creep"); then fit (exact title or name, an artist's songs and albums for their exact name, or artist and title together; else every typed word starts a word of the row; else most of them: two or more, and two thirds of the words that count (numbers, and words of four letters or more: "of" and "the" don't); else loose). Words found only in a "(from …)" note count as most: "You Can't Run From Me (from Cyberpunk: Edgerunners Season 2)" is from the work typed, not that work, then popularity in tenths, then the listener's own (saved, liked, a followed artist), then exact popularity. Popularity is a log scale per source: Deezer rank (1e6 = 1), Deezer fans (1e7), Last.fm listeners (tracks 5e6, artists 1e7). Deezer albums have no count, so their order stands in. MusicBrainz rows have none.
- **Misspelt and borrowed artists:** a Last.fm-only artist one or two edits from a confirmed artist (at most one per four letters), with under 1% of its fans or listeners, isn't listed ("Radiohed" next to Radiohead). Nor is an artist whose name holds a confirmed artist's and more ("Radiohead - Topic", "We Love Taylor Swift", "SZA with Kendrick Lamar") with under 1% of its audience (none known counts as none), or Last.fm-only whatever its count: a credit string, not an artist with releases.
- **Famous near matches:** an artist with under a tenth of the fans or listeners of a confirmed artist that holds every typed word or is a typo away ranks at that artist's fit, so popularity decides ("mozart" is Wolfgang Amadeus Mozart before a pop act called Mozart; "radiohed" is Radiohead before a namesake MusicBrainz knows).
- **Titles and "the":** a leading "the" is ignored on either side, for titles as for names ("dark side of the moon" is exactly "The Dark Side of the Moon").
- **Top result:** the best of the first artist, album and track: confirmed before Last.fm-only, then fit, then popularity, with artists and full-length albums (album, compilation) leaning +0.15, and EPs and singles +0.05 ("radiohed" is Radiohead, "creep" is the song, "cyberpunk edgerunners season 2" is the soundtrack "Cyberpunk: Edgerunners 2").
- **`search.catalog`** names the 50 best tracks once every source is in (at the answer when all were in by then).
**Deezer withholds its catalog where it isn't open.** From the homelab's IP, `/search/track` and `/search/album` answer `data: []` with `total` above zero, and `/infos` says `open: false`. Only `/search/artist` and lookups by id work. Should Deezer be searched for tracks there anyway (before `/infos` has answered), that is named in `errors.deezer` ("isn't open in this country") rather than taken as "nothing found".

## Pictures (`GET /api/catalog/art/{id}?size=96|256|512`)

- **Sources:** Deezer covers and artist photos (any size, from `md5_image` / `picture_xl`), iTunes covers (any size, from `artworkUrl100`), Cover Art Archive covers for MusicBrainz albums, and Wikimedia Commons artist photos (`Special:FilePath/<file>?width=`). Last.fm is never a picture source.
- **Every album discovery hands out** (search, feed, album, artist and tag pages) with a release-group MBID and no other source gets the Cover Art Archive's (`art::picture`), so new releases have covers.
- **Fetching:** a request waits at most 3 s for a picture lull doesn't have yet. Meanwhile it answers 404 with `Cache-Control: no-store`, and the fetch goes on in the background: one try of up to 20 s, one fetch per picture and size, shared by every request for it. The next request is served from the cache. A source that fails leaves nothing cached, so it is asked again next time.
- **The URL is relative** (`api/catalog/art/…`): the UI's routes are all in the hash (`#/artist/…`), so it resolves the same from every screen and behind a reverse-proxy sub-path. The archive is asked for its smallest size that covers the request (250 for 96 and 256, 500 for 512), and every picture is scaled down to the size asked for (JPEG). `size` defaults to 256.
- **Where each picture comes from:** albums, `lull_core::catalog`'s `catalog_albums.art` (a URL with `{size}`), recorded by search; artists, only `discovery_artist_photos` ([Artist pages](#artist-pages)). The `catalog_artists.image` search records is never served.
- **Entities:** albums carry `art` and artists `image` (with `?v=`, for a vetted photo only), `api/catalog/art/<id>` relative to the page (the UI adds `?size=`), only when a source is known and hasn't said lately (a day) that it has none: a new release the Cover Art Archive has no cover for gets no `art` once that is known, instead of a 404 on every load. Albums without one use library's `/api/art/{albumId}`.
- **Cache:** `<data dir>/catalog-art/<id>-<size>`, 30 days; "the source has none" for a day.

## Share links (`GET /api/share/{kind}/{id}`)

`share.rs`: a public link and cover for a track, album or artist, used by the share menu and as the desktop app's Discord Rich Presence art.
- **Songs and albums:** one iTunes Search call for `artist title` (`entity=song` or `album`, 10 results). The first listing by the same artist (or a credit including them) with the same title wins, else the same title loosely (`ids::loose_title`). The link is Odesli's page for that iTunes id: `https://song.link/i/<trackId>` or `https://album.link/i/<collectionId>`, which lists every streaming service.
- **Why not Odesli's API:** its keyless API answers 401 `PUBLIC_API_ACCESS_DEPRECATED` (checked 2026-11-02). Its pages still resolve an iTunes id, so lull builds the page URL and makes no Odesli call. `song.link/i/<collectionId>` and `artist.link/i/<artistId>` don't resolve; `album.link/i/<collectionId>` does.
- **Artists, and anything the store doesn't list:** the Last.fm page (`https://www.last.fm/music/<artist>[/_/<track> | /<album>]`), `source: "lastfm"`.
- **Artwork:** the store's cover at 600 px (`…/600x600bb.jpg`), else the public source the catalog knows for the album or artist (Deezer, iTunes, the Cover Art Archive) at 600 px, else `null`.
- **Cached** in `discovery_cache` (`share:<kind>:<id>`): store links 30 days, Last.fm fallbacks 1 day, so a release that reaches the store later gets its link. An expired answer is served when iTunes fails. One link is resolved at a time, so the same link asked for twice at once costs one call. iTunes calls share the 18-a-minute budget with search; with it spent and nothing cached the answer is 503.

## Playlists in search

`search/playlists.rs`. For 3+ characters, search also asks for playlists people made: Deezer's `search/playlist` (15, about 0.4 s from the homelab: a fast source, in the first answer) and YouTube's search filtered to playlists through yt-dlp (`Youtube::search_playlists`, 12, a few seconds: usually a `search.more`). YouTube Music's own search lists the same community playlists, without names, so YouTube's stands for both. Both are keyless and kept a day (`dz:playlists:`, `yt:playlists:`), so the same words later get YouTube's in the first answer. There is no Spotify: its search needs a key.
- **Relevance first:** the title against the words typed (fit as album titles fit, but a title that is just the words counts as one holding every word, since anyone can name a playlist anything; a playlist holding less than most of the words is left out), then soundtrack wording ("soundtrack", "OST", "score", "songs from", "songs used", "list of songs", "tracklist"…).
- **Then size and popularity:** 0.4 × the site's own order (1/(1 + place/3)) + 0.3 × size (√(songs/100)) + 0.3 × following (log fans, 100 000 full); unknown size or following counts 0.4 / 0.3. YouTube's search gives neither, so its order stands in for them. Deezer's fans come only with a playlist's whole page (seconds for a long one), so an answer carries the fans already known and the first 4 unknown are fetched after it (inside Deezer's window, kept a week), for the next search.
- **Top result:** a query names a film, show or game when two or more playlists holding every word say soundtrack. Then the album holding every word that says soundtrack takes the top ("cyberpunk edgerunners 2" is still its OST album), and without one, the best such playlist (`top.kind: playlist`; "person of interest"). Otherwise search's own top stands. Playlists arriving later (`search.more`) never change the top.
- **Pictures** go through `/api/catalog/art/pls-<link in hex>`, their source kept a month in `discovery_cache`, so the CSP stays same-origin.
- Opening one previews it without importing; collection.md#outside-playlists.

## Matching another service's songs (the `Catalog` seam)

Collection's playlist link imports ask `Catalog::find_song` for each song a Spotify, Deezer, YouTube or SoundCloud playlist names (`search/matching.rs`).
- **Order:** the library's copy (search's library half, `artist title`); then Deezer's `track/isrc:<isrc>` when the source gives one (it answers in every country, within Deezer's 50 per 5 s, so an ISRC playlist matches in seconds); then by name: Deezer `search/track` (10) where it is open, else iTunes `entity=song` (10). An ISRC match takes Deezer's album naming, which can differ from iTunes search's (another id for the same song).
- **Same song:** search's rule (`merge::same_song`: a shared ISRC, or loose artist and title with lengths within 3 s), tried with the whole credit and each name in it (`radio::credited_names`, the reading radio uses: "Daft Punk, Pharrell Williams", "A feat. B"), so a credit with a comma in a name ("Tyler, The Creator") still matches as a whole. The first hit in the catalog's order that is the same song wins.
- **Recorded** like a search row (track, album with its cover, catalog source), so the id resolves and the album page opens.
- **Limits:** iTunes match lookups spend only the calls the listener's searches leave spare (12 of 18 a minute), waiting for one rather than skipping, and are kept 7 days (`it:match:`). Deezer name lookups are kept a day, ISRC lookups 30 days, inside Deezer's shared window.

## Ids

Album and artist ids are hashes (`lull_core::ids`), so discovery remembers every album and artist it shows (`discovery_albums`, `discovery_artists`), and every album's artist with it (with the MBID the release names), so an album's artist link resolves. An unknown id falls back to the catalog: the first track with that `albumId` / `artistId`. Tracks go to the catalog. An id the catalog already has keeps every fact it has, because another crate may have added file facts. Discovery only fills its gaps: year, number, disc, length and tags.
