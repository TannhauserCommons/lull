# Scrobbling and Last.fm auth

Owner: Discovery session (`crates/lastfm`). Needs `LULL_LASTFM_API_KEY` and `LULL_LASTFM_API_SECRET` (an API account from last.fm/api/account/create); without them the `lastfm` service is `off` and every endpoint answers 503.

## Sign-in (web auth)

1. **`GET /api/lastfm/connect`** (a browser navigation, not fetch) redirects to `https://www.last.fm/api/auth/?api_key=…&cb=<LULL_PUBLIC_URL>/api/lastfm/callback?state=<nonce>`.
   - The nonce is 128 random bits, single use, valid for 15 minutes.
   - Without `LULL_PUBLIC_URL` the answer is 503.
2. **Last.fm sends the browser back** to `/api/lastfm/callback?state=…&token=…`. The token is valid for 60 minutes and can be used once.
3. **`auth.getSession`** (signed) trades the token for a session key. The key lasts until the listener revokes access, and is stored in `lastfm_session` with the user name.
4. **The browser lands on `/#/settings/lastfm`.** A failure is kept and shown as `error` in `GET /api/lastfm`.

- **Signature:** `api_sig` is the md5 of every parameter, as `name+value` in name order, excluding `format` and `callback`, followed by the secret.
- **Writes are POSTs.**
- **With `LULL_ACCESS_TOKEN` set:** the callback is a cross-site navigation from last.fm. The token cookie is `SameSite=Lax`, so it still reaches the gate.

## When a play scrobbles

The rules are Last.fm's own (`rules.rs`, tested):
- The track is longer than 30 s.
- It was played for at least half its length, or 4 minutes, whichever comes first.
- With the length unknown (0), only the 4-minute rule applies.

**Flow:**
- `play.started { playId }` sends `track.updateNowPlaying`. It is best effort and never retried, as Last.fm asks.
- `play.ended { playId, at, playedS, duration, artist, title, album }` is judged by the rules. Collection's `at` is the play's start, Last.fm's timestamp.
- An eligible play is queued with `timestamp = at`.
- Each play publishes `scrobble { playId, status: pending | short }`, and `done` once sent. History uses the same words. `short` means Last.fm's rule failed: a skip after the half still scrobbles.
- With the "scrobble what I play" switch off, nothing is queued or published.

## The queue (`queue.rs`, tested)

- **`lastfm_queue`, one row per play (`play_id` primary key).** A repeated `play.ended` is ignored.
- **Sent oldest first, in batches of up to 50 (`track.scrobble`).** A row is deleted only once Last.fm took the batch. Plays Last.fm *ignores* (too old, filtered, daily cap) count as taken and are logged.
- **A failed batch waits: 30 s, doubling, at most 1 h.** It is retried for as long as it takes, across restarts and offline spells.
- **"Retry now"** (`POST /api/lastfm/retry`) makes every row due at once.
- **Error 9 (session revoked):** state `warn` with "connect again". The queue is kept, so reconnecting sends it.
- **Disconnect** (`DELETE /api/lastfm`) forgets the account and drops the queue. The UI warns first.

## Status (`ctx.status` "lastfm", `extra: { queued }`)

| State | When |
|---|---|
| `off` | not configured, or not connected |
| `ok` | connected ("Connected as …", or "scrobbling paused") |
| `warn` | scrobbles waiting after a failure ("N scrobbles waiting, retrying in 40 s"), or the session was revoked |

## History as a taste signal

- **When it's published:**
  - at start;
  - on connect;
  - when "use my history" changes;
  - daily.
- **What it carries:** the listener's top 100 artists of 12 months (`user.getTopArtists`), as a `lastfm.history { user, artists: [{ name, plays }] }` event.
- **Who reads it:** discovery stores it as a taste prior (see [taste.md](taste.md)). With the switch off or disconnected, the event carries an empty list.
