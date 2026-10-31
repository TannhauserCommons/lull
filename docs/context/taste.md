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

**Fading:** a signal of age `d` days counts `0.5^(d/45)`, a 45-day half-life. Plays older than 365 days are not read; they would count under 0.4%.

## Artists and tags

- **Artist weight:** each signal adds its weight to its artist (`ArtistId::of(name)`).
- **Tag weight:** the same weight is shared evenly among the track's tags (catalog), or the artist's tags (from artist pages) when the track has none. Following a tag adds +4.0 to that tag.
- **Umbrella genres** (rock, pop, electronic…) stay in the profile, but picks and radio never seed from them (see [discovery.md](discovery.md)).
- **Hidden artists** (`hide-artist`) are left out of the model and filtered from the feed and radio.
- **Negative artists:** an artist below zero (mostly skipped, less-like) is never picked.
- **Known tracks:** tracks already played or liked are not offered as picks.

## Where it is used

- **Feed:** the top artists seed similar-artist picks. The top tags and followed tags seed tag picks and the tag-matched new releases. Liked artists' fresh releases rank first. See [discovery.md](discovery.md).
- **Radio:** taste nudges liked artists up (familiar) or down (adventurous).
- **Stats:** `GET /api/taste` returns `{ artists: [[artistId, w]], tags: [[tag, w]], entities }`. These are the positive weights only, scaled so the strongest is 1 and rounded to 2 places.

## Reset

`POST /api/taste/reset` stores `taste_reset_at = now`, deletes saves and feedback, and ignores plays and likes from before it. Follows, hidden artists and the Last.fm history stay. They are standing choices, undone where they are made: unfollow, Settings > Last.fm "use my history".

Tests (`taste.rs`): decay at one and ten half-lives, follows not fading, completion vs skip weights, tag sharing, hidden artists.
