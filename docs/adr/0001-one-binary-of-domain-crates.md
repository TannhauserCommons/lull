# ADR-0001: One binary built from domain crates that meet only through core

- Status: accepted
- Date: 2026-10-31

## Context

lull is a single-user server with many concerns: Soulseek, Jellyfin, discovery, Last.fm, lyrics, the listener's collection and skins. Several agents build it at the same time. They need boundaries that keep each one's work readable on its own, and that stop two of them from editing the same files.

## Decision

- **One Rust binary, one crate per domain.** A Cargo workspace with one crate per domain, plus `core` (shared contracts) and `server` (wiring).
- **Domain crates depend only on `core`.** The few calls that cross domains are two traits in `core::seams`: `Library` and `Transfers`. Everything else crosses through the event bus.
- **Every crate exposes the same shape** (`MIGRATIONS`, `start`, `router`). `server` wires all of them in one place.
- **One SQLite database.** Each crate owns its tables. `core` owns the shared ones: plays, likes, follows, and the catalog of every track shown.
- **Content-derived ids.** A song's id is a hash of its normalised artist, album and title. A Soulseek result and the same song saved in Jellyfin share one id, so likes, plays and playlists survive a save.
- **The web UI stays buildless,** as the design package is, and is embedded in the binary. Webamp is the one npm package, pinned and vendored at build time.

## Consequences

- **Parallel work is mostly conflict-free.** Owners edit disjoint folders; `Cargo.lock` is the only expected conflict, and it's mechanical.
- **Shared code changes carry a rule.** Adding to `core` affects everyone, so changes there are additive only.
- **More manifests to keep.** Nine manifests instead of one: workspace dependencies keep versions in one place.
- **Ids can't be reversed.** That's why the catalog exists.
- **The single-process design rules out horizontal scaling,** which a single listener doesn't need.
