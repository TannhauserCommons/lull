# ADR-0008: Search the catalogs; pick Soulseek copies by the listener's rules

- Status: accepted
- Date: 2026-11-01

## Context

Search is Soulseek-first today.
- It shows songs guessed from file paths, grouped by peer folder, after a search of up to 15 s.
- The library half comes back separately.

The listener wants search to work like Spotify's: artists, albums, singles and tracks with real art, quickly, and never missing a niche track. Pressing play should pick a Soulseek copy by rules they set. Saving should have its own rules, and every rule should be a setting (2026-11-01).

Exploring the code found three gaps:
- the stored Settings › Sources values were never applied;
- song-to-file matching was exact-only, with no duration check;
- a save with no known source failed instead of searching.

## Decision

**Search is a catalog search.** Discovery owns `/api/search` and merges four sources:
- **Deezer** (no key; 50 requests per 5 s). It answers first, with art for artists, albums, singles and tracks, plus ISRCs and durations.
- **MusicBrainz:** one search per query through the shared `Http::pace` budget. Covers come from the Cover Art Archive.
- **Last.fm** (`track.search`, `artist.search`): independent and scrobble-known music, plus listener counts. It's never the art source.
- **Saved songs** through the `Library` seam, badged as saved.

**Merging and ranking:**
- **One row per song:** matched by ISRC when both sides have one; otherwise by normalized artist and title (feat., version, remaster and edit tags stripped) with durations within ±3 s.
- **Ids** stay content-derived (ADR-0001).
- **Each merged row** keeps the best art and the most complete facts.
- **Order:** exact matches first, then popularity (Deezer rank, Last.fm listeners), then the listener's follows and likes.
- **One top result:** an artist, album or track.

**Fast first, more later.**
- The response carries what arrived within about 400 ms (Deezer and the library).
- MusicBrainz and Last.fm additions follow as a `search.more` event, merged by the UI without moving rows the listener is reading.
- Queries are debounced by the UI and cached by discovery for 10 minutes.

**Soulseek behind the catalogs:**
- **Nothing is missed.** A submitted query (3+ characters) still runs a Soulseek search. Songs no catalog knows come back as "Only on Soulseek" rows built from file tags.
- **Instant play.** Discovery publishes `search.catalog { query, trackIds }` for the top tracks, and soulseek pre-warms their sources, paced.
- **Raw files.** Folder and file results appear only in the source picker, unless Settings › Search › "Show file results" adds a Files tab.

**Source rules: two profiles.** The types are `lull_core::quality::{StreamProfile, SaveProfile}`; collection stores them and publishes them in `settings` (ADR-0050).

| Field | Meaning |
|---|---|
| `minimum` | `{ losslessOnly, kbps, formats }`. Lossless passes any lossy floor. A lossy file of unknown bitrate fails a floor. An empty `formats` means any |
| `pick` | `fastest` (best score among copies that pass) or `highest` (best quality among copies that pass, speed breaking ties) |
| `below` | Streaming: `playBest` (marked below the minimum), `keepLooking` or `ask`. Saving: `saveBestUpgrade`, `wait` or `ask` |
| `autoUpgrade`, `upgradeEveryH` | Saving only: re-search saved songs below the minimum and replace them when a passing copy appears |

- **Defaults:**
  - streaming: 192 kbps or lossless, `fastest`, `playBest`;
  - saving: 320 kbps or lossless, `highest`, `saveBestUpgrade`, auto-upgrade daily.
- **Fallback:** a stalled or silent peer falls back to the next copy in the profile's order. The peer limits (maximum queue, stall timeout) are applied, not just stored.
- **One quality rule:** `lull_core::quality::Quality::beats` (lossless over lossy, or at least a fifth more bitrate) replaces the separate rules in soulseek (`dup_kind`, +64 kbps) and library (`items.rs::Quality`).
- **Matching:**
  - fuzzy artist and title matching with the same normalization as merging;
  - a duration check when the file reports a length;
  - a looser second query when the first finds nothing.
- **Saves search when no source is known,** as playback does.
- **Upgrades** are paced (at most one re-search a minute) and replace files through the existing replace path, keeping tags, cover and Jellyfin entry. Each one shows in Activity.
- **Switching source mid-song** reopens the stream from the chosen peer and seeks back to the current time.

## Alternatives

- **Spotify's Web API.** Since February 2026 development-mode apps need Premium and get capped search results, and its terms don't fit a downloader.
- **MusicBrainz alone:** complete for released music, but 1 request a second, sparse art and no artist photos.
- **Keeping Soulseek-first search:** slow, file-shaped results, and no art. It's what the listener asked to replace.

## Consequences

- **More upstreams.** Search depends on three outside services. Each one failing degrades to the others, and the status tray names it.
- **Deezer's limit is per IP,** fine for one listener with debouncing and caching.
- **Catalog entries now include albums and artists with art,** so `Entities` carries them for search results.

## Update 2026-11-01: the fast catalog is chosen per hub

- **Deezer is closed in some countries.** From the homelab, `/infos` says `open: false`, and track and album searches come back empty while still reporting a `total`. Artist search and lookups by id still work.
- **So each hub asks Deezer's `/infos`** at start and daily:
  - where Deezer is open, it answers tracks, albums and artists as above;
  - where it is closed, **Apple's iTunes Search** answers tracks and albums (one `entity=song` call per query, albums derived from the songs, the store from `LULL_CATALOG_COUNTRY`, default US), and Deezer answers artists and their photos.
- **iTunes' limit** (about 20 calls a minute) is kept by the 10-minute cache, the UI's debounce and a budget of 18 calls per rolling minute. Over budget, a search answers with the library and gets `search.more` from MusicBrainz and Last.fm: never an error.
- **The choice shows** as the `catalog` status ("catalog: iTunes (US) + Deezer artists").
- **Not chosen:** sending Deezer's calls through a proxy in another country.

## Update 2026-11-03: over the iTunes budget, calls wait their turn

- **Answering without iTunes was wrong.** Two people searching "edgerunners 2" on one hub, a minute apart, got different top results, and one never saw "High Forever": as-you-type searches had spent the 18 calls, the run answered with MusicBrainz and Last.fm alone, said nothing, and was kept 10 minutes for anyone searching those words.
- **Now a call over the budget waits its turn,** the newest search's songs first, then album searches and artist pages, then link imports. A search that gets no turn within its 8 s shows `errors.itunes`, and its run isn't kept.
- **And a run's answers are one list:** the first answer fixes the top result and order, `search.more` rows go after it, and later answers for the same words get that list, not a new ranking.

## Update 2026-11-04: Apple's order kept, Last.fm out of search

- **Measured on 63 queries,** Apple's song order was nearly always what the listener meant; lull's re-ranking threw it away (misspellings, unknown exact namesakes, live takes first). Ranking now forgives typos, counts a title being typed and an artist with words of the title as exact, ranks versions after the song, and, where iTunes gives no counts, takes a song its own artists have on more releases as the better known (discovery.md#search).
- **Last.fm's search is no longer a source:** it added covers without a release and scrobbles' credit strings, nothing the catalogs and MusicBrainz missed. Last.fm stays for bios, similar artists, covers and scrobbling.
- **Deezer stays for artists where it is closed:** Apple's song and album searches name no artists, and Deezer's fans are what rank them.
- **A search's iTunes album call takes the whole budget after its songs,** and the first answer waits for the songs: queued behind background work, it held the songs back past 1.5 s, and the run's first answer, without them, fixed a wrong list.
- **YouTube Music's search fills what Apple lacks,** in the first answer (about 0.5 s): one call per query to the endpoint its web player reads, as YouTube publishes no API. It only adds rows the catalog lacks, its listed ones after the catalog's of the same fit; its play counts rank the catalog's songs too, as Apple's order is a guess. A contract workflow reads its live answer daily, since its shape is YouTube's to change. Not chosen: MusicBrainz in the first answer (0.9 s median, often refused, and two calls with release groups), Discogs before a token exists to measure it.
