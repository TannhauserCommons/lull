# ADR-0020: The library is a persisted index of Jellyfin, and retagged saves keep their id through an alias

- Status: accepted
- Date: 2026-10-31

## Context

- **Who reads the saved library.** The UI's lists, saved badges on every Soulseek result (through `Library::saved`), the library half of search, and M3U matching.
- **What it must survive.** Jellyfin is on another machine and can be down. Lists then show the last known library, read-only.
- **The id problem.** Song ids are content hashes (ADR-0001). A save tags the file from MusicBrainz, which often corrects artist, album or title: Soulseek folder names carry "[FLAC]" or a remaster note, so the id changes. The design asks that a finished save flip the badge in place, and that likes and playlists survive the save.

## Decision

- **An in-memory index answers every read.** It is built from a full read of Jellyfin's audio items:
  - at start-up, every 30 minutes, after a rescan, and when Jellyfin comes back;
  - merged with the newest items while a save waits for its file.
- **It is persisted** in `library_items`, so lists and badges work at once after a restart and while Jellyfin is down (`stale: true`).
- **A saved song's id comes from its album artist** (the track artist for compilations), because that is what a Soulseek folder gives.
- **A save whose tags change the id records an alias** (`library_aliases`, asked id → saved id). Library answers the asked id with the saved song, under the asked id, and writes it into the catalog under that id too.

## Alternatives considered

- **Query Jellyfin per request** (search, `saved(ids)`, lists). Rejected:
  - it would put a network call on every search and every badge;
  - nothing would answer while Jellyfin is down.
- **Keep Soulseek's metadata when tagging, so the id never changes.** Rejected: that throws away the reason for matching, and Jellyfin would show folder-name noise.
- **Track artist as the id basis.** Rejected: "A feat. B" songs would get ids no Soulseek folder produces.
- **Rewrite likes, plays and playlists to the new id.** Rejected: it touches tables other crates own, and it breaks references held by the browser.

## Consequences

- **Lag.** Changes made in Jellyfin outside lull (deletions, retagging) show up within 30 minutes, or at once after a rescan.
- **Memory.** The whole library sits in memory: a few MB for tens of thousands of songs.
- **Two ids for one song.** A song saved under a corrected name is reachable by both ids. The library lists show the saved id; the old one resolves through the alias.
