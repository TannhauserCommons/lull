# ADR-0030: Discovery reads ListenBrainz, MusicBrainz and Last.fm; not Discogs or Rate Your Music

- Status: accepted
- Date: 2026-10-31

## Context

The feed, radio and the album, artist and tag pages need recommendations, similar artists, new releases, tracklists, tags and descriptions. The brief asks for niche and micro-genre music, not charts, with reasons the listener can read. Candidate sources were ListenBrainz, MusicBrainz, Last.fm, Discogs and LRCLIB (lyrics). Rate Your Music forbids scraping and has no API. The full comparison, with sources, is in the knowledge vault: `projects/lull/research/discovery-sources.md`.

## Decision

- **ListenBrainz is the engine.** It supplies:
  - fresh releases: every release of the last 30 days, about 10,000, with tags;
  - collaborative-filtering recommendations for a ListenBrainz user;
  - tag radio, with a popularity window, so lull can ask for the obscure end of a tag on purpose;
  - artist radio: similar artists in easy, medium and hard steps;
  - similar artists;
  - batch recording metadata (artist, release, length, tags in one call).

  Data is CC0, and none of these calls needs a token.
- **MusicBrainz for releases.** It supplies discographies, tracklists, release and artist tags, and "release groups tagged X since date D" for tag pages. It is the identity layer every other source refers to by MBID.
- **Last.fm, when an API key is configured.** It supplies:
  - artist bios and listener counts;
  - similar artists on artist pages;
  - tag descriptions, tag top artists and similar tags;
  - the listener's own history, which becomes a taste signal (see `docs/context/taste.md`).

  It isn't used for picks: its tag charts are popularity charts, which is what lull is meant to avoid.
- **Discogs is not used.** Its styles are a fixed list of about 600 and add little to user tags. Its terms would also shape the product:
  - nothing may be shown more than six hours after it was fetched;
  - every item needs a linked "Data provided by Discogs" credit and a non-affiliation notice;
  - search needs a token.
- **LRCLIB for lyrics** (lyrics crate). It's the only open, synced-lyrics API.
- **Politeness is built in:**
  - every answer is cached for a source-specific time in `discovery_cache`;
  - a stale answer is served when a source fails;
  - each source is paced (ListenBrainz 1/s, MusicBrainz about 1/s, Last.fm 2/s).

## Consequences

- **Works without a single key.** Picks, radio, new releases, tags and tracklists all work. Last.fm adds bios, listener counts and tag descriptions. A ListenBrainz user name adds "listeners like you" picks.
- **Niche by construction.** Tag picks come from the low-popularity end of a tag. New releases include releases with a few listens.
- **Slow first build, then cached.** MusicBrainz's 1/s and ListenBrainz's 1/s make the first feed build take tens of seconds. It runs in the background and is cached, so only the very first visit waits.
- **No Discogs settings.** `LULL_DISCOGS_TOKEN` and the Settings > Discovery switch for Discogs were removed with this decision.
- **Last.fm terms:** non-commercial use, and data is credited where shown (`via` on picks, the UI's attribution).
