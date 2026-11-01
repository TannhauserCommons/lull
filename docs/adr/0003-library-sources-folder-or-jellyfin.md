# ADR-0003: The library is a music folder or Jellyfin, switchable with nothing lost

- Status: accepted
- Date: 2026-11-01

## Context

v1 needs Jellyfin. lull 2 must work for people without it, let Jellyfin users keep using it, and let anyone switch between the two without losing likes, history or playlists (PROJECT.md).

## Decision

**Two sources behind the `Library` seam,** chosen by `LULL_LIBRARY` = `folder` | `jellyfin`. It defaults to `jellyfin` when `LULL_JELLYFIN_URL` is set, and `folder` otherwise.
- **Folder:** `library` indexes `LULL_MUSIC_DIR` itself.
  - It walks the folder and watches it for changes (`notify`).
  - It reads tags with lofty, which `tags.rs` already uses, into its own table.
  - Audio is served with range support (tower-http's file serving); covers come from the embedded art or `folder.jpg`/`cover.jpg`.
- **Jellyfin:** today's code. Connecting gets two conveniences:
  - **Quick Connect:** the listener approves a code in Jellyfin, and lull stores the resulting token as `LULL_JELLYFIN_API_KEY` through `core::settings`;
  - **discovery:** lull finds Jellyfin on the local network (UDP 7359).
- **The index is keyed by track id;** each entry knows where its audio lives (a Jellyfin item or a file path).

**Switching keeps everything.**
- Likes, plays, playlists and follows are keyed by content-derived ids (`core::ids`), never by source ids.
- On every switch, `library` reconciles the two sources **by file path**: Jellyfin reports each item's path, and the folder index has it. Where tags and Jellyfin metadata produce different ids for the same file, the pair goes into `library_aliases`, so nothing points at a track that has gone.

**Jellyfin mirror, one way.**
- When Jellyfin is connected, its favourites and playlists are imported once.
- After that, lull's likes and playlists are mirrored into Jellyfin (following the `like` and `playlist` events), so Jellyfin's own apps see them.
- lull stays the source of truth; nothing flows back, so there are no conflicts.

**Saves** go into `LULL_MUSIC_DIR` in both modes. Jellyfin is asked to rescan only in Jellyfin mode, and the folder watcher picks the file up in folder mode.

**The seam grows additively** (`core::seams::Library`): `artists`, `albums`, `album`, `stream` and `cover`, for the Subsonic API. They have defaults, so crates compile before `library` implements them.

## Alternatives

- **Always read the folder, even in Jellyfin mode.** One code path, but it breaks for a Jellyfin on another machine without a shared folder, and it ignores Jellyfin's metadata edits.
- **Two-way sync with Jellyfin.** It needs conflict rules for every field; one-way keeps lull authoritative.
- **Bundling Navidrome or another server for folder mode.** It's a second server to run, configure and update, for a job lofty and a file watcher already do.

## Consequences

- **Two sources to maintain.** Both sit behind one seam and one index type.
- **Dropped likes are logged.** A file whose tags change after it was liked is matched again by path; one with no path match (deleted) drops its like out of the library views, and the drop is logged.
- **No Jellyfin needed for CI.** Folder mode lets CI and the visual tests run against a fixture folder.
