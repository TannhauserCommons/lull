# ADR-0005: Offline downloads with an outbox, and lull Connect

- Status: accepted
- Date: 2026-11-01

## Context

The hub is the single source of truth (ADR-0002), but phones leave home and lose signal. Spotify users expect two things: downloaded music that plays anywhere, and controlling one device from another.

## Decision

**Offline downloads.** The listener marks songs, albums, playlists or Liked to keep on a device.
- **One interface in the web UI, two backends:**
  - **In a browser:** Cache Storage, with `navigator.storage.persist()`.
  - **In the Android app:** app storage, through the native player plugin (ADR-0006).
- The audio, the cover and the track's JSON are kept together. A downloaded track plays from local data:
  - a Blob URL in the browser;
  - a file path in the native player.

  So nothing has to slice ranges out of a cache.
- What a device keeps follows its source: a downloaded playlist updates when the playlist changes and the device is online.

**Outbox.** Offline, the web UI records changes in IndexedDB, each with a client-made id:
- plays;
- likes and unlikes;
- playlist edits;
- follows.

On reconnect it sends them in order to `POST /api/sync`, which applies them **idempotently**: an id already applied is acknowledged and not applied again. Plays keep their real `startedAt`, so Last.fm scrobbles carry the time the song was played.

**lull Connect.** Every paired device keeps an SSE connection; that is how the hub knows it's online.
- **Each device reports its state** to `POST /api/devices/me/state`: track, position, playing, volume and queue. The hub republishes it as `device.state`.
- **Commands:**
  - Any device can send `POST /api/devices/{id}/command` to play, pause, seek, skip or set the volume. The hub forwards each one as a `device.command` event to that device.
  - "Play here" transfers the queue and position, then pauses the other device.

## Alternatives

- **A service worker that answers range requests from cache.** It's the usual approach, but Blob URLs and native files avoid the range logic and work the same way in the app.
- **Conflict-free replicated state.** Unneeded: every change is additive or last-writer-wins on the hub.

## Consequences

- **Offline edits apply late.** A playlist edited offline on two devices applies both sets of edits in arrival order. That's acceptable for one listener.
- **Downloads need room.** They count against device storage, and the Downloads screen shows the size and lets the listener remove them.
