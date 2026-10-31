# Discovery: sources, limits and how the feed is built

Owner: Discovery session (`crates/discovery`). Source choice and why: [ADR-0030](../adr/0030-discovery-sources.md). Taste model: [taste.md](taste.md).

## Sources and their limits

| Source | Used for | Limit lull keeps | Cached | Auth |
|---|---|---|---|---|
| ListenBrainz `api.listenbrainz.org/1` | fresh releases, CF recommendations, tag radio, artist radio, recording metadata | 1 call/s (server allows 30 per 10 s per IP; 429 beyond) | fresh releases 6 h; radio and recommendations 1 day; recording metadata 30 days | none; `LULL_LISTENBRAINZ_TOKEN` sent when set; `LULL_LISTENBRAINZ_USER` enables recommendations |
| ListenBrainz labs `labs.api.listenbrainz.org` | similar artists | same pace | 7 days | none |
| MusicBrainz `musicbrainz.org/ws/2` | finding artists and release groups by name, discographies, tracklists, tags, tag + date search | 1.1 s between calls (1/s per IP average; going over gets every call a 503). Other lull crates share the IP | lookups 30 days; discographies 7 days; tag searches 12 h | none; User-Agent `lull/<ver> ( repo URL )` from `lull_core::http` |
| Last.fm `ws.audioscrobbler.com/2.0` | artist bio and listeners, similar artists (artist pages), tag description, tag top artists, similar tags | 0.5 s between calls (no published number; "several per second" risks suspension) | 7 days (tag text 30) | `LULL_LASTFM_API_KEY`; off without it |

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

## Ids

Album and artist ids are hashes (`lull_core::ids`), so discovery remembers every album and artist it shows (`discovery_albums`, `discovery_artists`). An unknown id falls back to the catalog: the first track with that `albumId` / `artistId`. Tracks go to the catalog. An id the catalog already has keeps every fact it has, because another crate may have added file facts. Discovery only fills its gaps: year, number, disc, length and tags.
