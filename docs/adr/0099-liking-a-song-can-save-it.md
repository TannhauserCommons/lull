# ADR-0099: Liking a song can save it, and only one song at a time

- Status: accepted
- Date: 2026-11-05

## Context

Listeners asked for liked songs to land in the library without pressing Save as well. A like comes from many places: the web's player bar, rows, Now Playing, menus and keyboard; the desktop tray, mini player and Classic; Android (the same web UI); the native Classic player, which calls the hub itself; Subsonic clients; likes made offline and synced later; and the Jellyfin favourites import. A save takes disk space and peers' bandwidth, and many listeners like songs only to shape their taste. One action can like hundreds of songs (multi-select, a Subsonic client starring its whole list), and those must not start hundreds of downloads unasked.

## Decision

- **A setting, off by default:** `library.autoSaveLikes` in `/api/settings`, the switch "Save songs you like" in Settings › Library and saving.
- **On the hub, in `likes::set`:** every like, from every client, passes it. A song newly liked on its own publishes `saves.request`, which library queues as Save would: the saving profile picks the copy (the streamed one when it passes), a saved song is skipped, a running save kept. A re-like changes nothing.
- **Decided by the request, never by timing.** One like of one song saves. One action liking many is a batch (`PUT /api/likes/{id}?batch=true`, a sync op's `batch`, the `Collection` seam given several songs) and saves none. The web asks once, "Save these N too?". Clients that can't ask (Subsonic) leave the songs as "liked, not saved". Offline taps synced later are single likes, and save.
- **Earlier likes are never saved silently.** Turning the setting on asks once; Settings lists "Liked songs not saved: N · Save them" (`GET /api/likes/unsaved`, `POST /api/likes/save`, explicit only).
- **Unliking never deletes** a saved song or cancels a save; the Activity row can cancel one.
- **Nothing new for failures:** a save from a like is an ordinary save job, with its Activity row, progress, retry and waiting.

## Rejected

- **In each client:** the native Classic player, Subsonic and synced offline likes would miss it, and every client would repeat it.
- **A time-based burst guard** ("more than 10 a minute"): it guesses at intent, and a quick listener tapping through an album would lose saves they wanted.
- **On by default:** a like is often only taste, and saving costs disk and peers' bandwidth.
