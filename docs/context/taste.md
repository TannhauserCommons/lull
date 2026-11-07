# Taste model

Owner: Discovery session (`crates/discovery/src/taste.rs`). Small, on-device and explainable (PROJECT.md non-goals): every weight is a sum of the signals below, and nothing else.

## Signals and weights

| Signal | Source | Weight | Fades |
|---|---|---|---|
| Play reached the end | core `plays` (`completed`) | +1.0 | yes |
| Played past half, not to the end | `plays` | +0.5 | yes |
| Skipped before the scrobble point | `plays` (`skipped`) | −0.5 | yes |
| Like | core `likes` | +3.0 | yes |
| Save done | `save` event → `discovery_saves` | +2.0 | yes |
| More like this / less like this | `POST /api/feedback` → `discovery_feedback` | ±3.0 | yes |
| Followed artist or tag | core `follows` | +4.0 | no |
| Last.fm history (top 100 artists, 12 months) | `lastfm.history` event → `discovery_history` | up to +2.0, log of plays relative to the top artist | no (refreshed daily) |
| The saved library (every saved artist; a compilation credits the artists on its saved songs, never "Various Artists") | `Library::albums` at each taste build | up to +1.0, log of saved albums relative to the artist with most: about one song heard to the end, so a library is a taste before anything is played and listening soon leads | no (it is what is saved now) |

**Damping** ([ADR-0100](../adr/0100-picks-stable-at-the-core-fresh-at-the-edge.md)): a day of listening counts, not each play. At most 3 heard plays and 2 skips per artist per day count. An artist is *established* (seeds picks, counts for a tag's 2 liked artists) once heard on 2 separate days, or acted on: a like, a save, a follow, "more like this", saved albums, the Last.fm history. An artist is held back from picks and radio for skips only after skips on 2 or more days, or after "less like this". So a binge day or a stray play barely moves the picks, and coming back is what does. "Various Artists", "VA", "Various" and "Unknown Artist" are no artists (`lull_core::ids::is_pseudo_artist`): signals on them weigh nobody, whatever was stored. Home's new releases count an artist as liked only when the listener listened to or chose it (`Taste::listened_artists`: established, less those only a saved album establishes).

**Fading:** a signal of age `d` days counts `0.5^(d/45)`, a 45-day half-life. Plays older than 365 days are not read; they would count under 0.4%.

## Artists and tags

- **Artist weight:** each signal adds its weight to its artist (`ArtistId::of(name)`).
- **Tags an artist carries:** those of its tracks the signals name (the catalog's, else Last.fm's `track.getTopTags` for that artist and title), or its own for a signal whose track has none, or that names no track. An artist's own tags are MusicBrainz's by its MBID, and only by an MBID its [ids](discovery.md#artist-identity) hold. **Identity is ids, never names:** a name's tags may be any namesake's (Last.fm's artist pages are per name, so its artist tags never count), and the listener's own songs decide who their artists are before any recommender; nothing may fall back to a name.
- **Tag weight:** the sum of the weights of the artists carrying it, so a tag weighs how much of what the listener likes it runs through, however many other tags those artists have. Following a tag adds +4.0 to that tag. (Sharing a signal's weight among its tags made one song tagged only "rapper", or one artist's two labels, the strongest tags.)
- **Tags dug into** (`Taste::dig_tags`, for picks, radio starters and tag-matched new releases): specific ones carried by at least 2 liked artists (`TAG_ARTISTS`), plus followed tags. One artist's labels are that artist, often a namesake's.
- **Umbrella genres** (rock, pop, electronic…) stay in the profile, but picks and radio never seed from them (see [discovery.md](discovery.md)).
- **Hidden artists** (`hide-artist`) are left out of the model and filtered from the feed and radio.
- **Negative artists:** an artist below zero (mostly skipped, less-like) is never picked.
- **Known songs:** songs already played or liked are not offered as picks, on any release and by any artist of the credit (`ids::loose_artist` + `loose_title`, credits split as radio splits them), and a fresh release is left off the shelf when a song of it, or the song a single is named after, is known.

## Where it is used

- **Feed:** the top artists seed similar-artist picks. The top tags and followed tags seed tag picks and the tag-matched new releases. Liked artists' fresh releases rank first. See [discovery.md](discovery.md).
- **Radio:** taste nudges liked artists up (familiar) or down (adventurous).
- **Stats:** `GET /api/taste` returns `{ artists: [[artistId, w]], tags: [[tag, w]], entities }`. These are the positive weights only, scaled so the strongest is 1 and rounded to 2 places.

## Reset

`POST /api/taste/reset` stores `taste_reset_at = now`, deletes saves and feedback, and ignores plays and likes from before it. Follows, hidden artists, the Last.fm history and the saved library stay. They are standing choices, undone where they are made: unfollow, Settings > Last.fm "use my history".

Tests (`taste.rs`): decay at one and ten half-lives, follows not fading, completion vs skip weights, tags counted by the liked artists carrying them, known songs across releases, hidden artists.
