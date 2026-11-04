# ADR-0033: One picture service in core, driven by what is on screen

- Status: proposed
- Date: 2026-11-04

## Context

Pictures appear slowly and sometimes not at all. Scrolling a long list fast is the worst case: the screen the listener stops on fills last. On the live hub (599b843) the causes were:

- **Nothing knows what is on screen.**
  - Library covers start a lookup per request that nobody cancels.
  - Catalog covers go through a first-in first-out queue, dropped only after 120 s, in the order answers listed albums by id.
  - Bare songs (imports) are resolved oldest first, one at a time.
  - Artist photos are vetted from a 64-deep queue that drops new artists when full.
  - Rows scrolled past keep their place ahead of the rows on screen.
- **Turns are booked, not taken.** `Http::pace` and `Http::window` book a slot when a call is asked. A hundred covers scrolled past book a hundred MusicBrainz seconds, and search's calls queue behind them.
- **Slow and repeated lookups.**
  - Library's name lookup is a MusicBrainz release-group search per album and size, uncached, then the Cover Art Archive.
  - Measured from the homelab on 2026-11-04 (41 albums and 43 artists a listener played, liked or was offered; each source asked once at its published pace), the archive takes 3.0 s (median) and 8.5 s (p90) for a 250 px thumbnail.
  - Last.fm `album.getInfo` finds 76% of the albums in 0.5 s, and its CDN serves the picture in 0.07 s.
  - iTunes finds 59% in 0.09 s, but lull has only about 18 iTunes calls a minute, and search needs them.
  - MusicBrainz's name search found nothing the others missed.
- **Vetted artist photos are never announced,** so one shows only after the page is read again.
- **Two caches and two queues** (`library/art.rs`, `discovery/art.rs`) do the same outside work with different rules.

## Decision

### One service, in core

`lull_core::pictures::Pictures` is the hub's one service for **outside pictures**: covers and artist photos that come from another site. It is a new `Ctx` field, so every crate shares one instance.

It owns:
- the cache, `<data dir>/catalog-art/<id>-<size>` (discovery's today) and "none" (`<id>.none`);
- one job per picture (single-flight);
- the order jobs run in, and the outside calls they make;
- `art.ready`.

A crate tells it where a picture comes from through a `Finder` that discovery registers at start:
- the source it already knows (`catalog_albums.art`, a vetted photo, a playlist's picture);
- or the steps to find one by name.

Library keeps only the pictures it has itself (Jellyfin's image, the folder's file or embedded picture). They are instant and never queued. For an album without one, it asks `Pictures`, so Subsonic's `getCoverArt` gets the same outside cover as the web.

### The instant path

- A cached picture, a local one and a remembered "none" are answered at once.
- A cache hit never creates a job or waits behind one.

### The screen sets the order

- The web UI's cover observer reports what is on screen: `PUT /api/pictures/screen { gen, visible, near }`.
  - It sends a report whenever that changes, at most 4 times a second, and once more when scrolling stops.
  - Each entry is a picture (`[route/id, size]`, e.g. `["catalog/art/al-…", 256]`), top to bottom.
  - Waiting covers (the hub said "not yet") and artists drawn without a photo are reported too.
  - `gen` grows with every report from that device; an older report arriving late is ignored.
- A job's rank is its best place on any device's latest screen:
  1. **playing:** the playing song and the next 3, from `device.state`;
  2. **visible;**
  3. **near:** within a screen of view;
  4. **asked:** a request is waiting on it, but no screen shows it, as with a Subsonic app;
  5. **background.**
  
  Within a place, the newest screen wins, then the order on screen.
- **Off-screen work is dropped before it starts.** A job that is on no screen, not playing and has no request waiting is removed when it would run, and whenever a new report arrives.
- **The screen also starts work.** For each picture reported that isn't cached or known to be missing, the route's owner asks for it. So a cover that scrolled away and back is fetched again, with no request from the browser.

### Lanes, and turns taken at send time

- **Jobs wait in lanes** by the outside service they call next, each with its own workers:
  - `fetch`: the source is known, so the picture is fetched from a CDN (8 workers);
  - `lastfm`, `itunes` and `vet`: 1 worker each.
  
  A slow lane never holds up another.
- **A finder runs in steps.** Each step names the lane of the next:
  1. Last.fm `album.getInfo`;
  2. iTunes, from search's spare budget only (`Need::Background`);
  3. the Cover Art Archive by release-group MBID.
  
  A found source is recorded once (`catalog_albums.art`), so every size and every later request reuses it, and the name lookup is never repeated per size.
- **`Http::pace` and `Http::window` take a turn when the request is sent,** not when it is asked. Each service has one fair async gate, and a caller that gives up leaves it with nothing booked. This applies to every crate, search included.
- **CDNs** have no published limit. They get at most 4 requests at once per host (`Http::host_place`); every source host speaks HTTP/2.
- **Retries** go back into the lane after 20 s, then 40 s, and still take their rank from the screen.

### One fetch per picture

- The job fetches the largest size wanted while it waited, writes it, and scales every smaller served size from it. A row (96), a tile (256) and a hero (512) of one album cost one outside fetch.
- A larger size asked later is a new job.

### Artist photos

- Vetting (Wikidata/Commons, else Deezer, compared against the artist's covers as now) takes the 12 best-ranked artists waiting, on screen first. The fixed 64-deep queue goes.
- A photo found is announced: `art.ready { keys, images: { artistId: url } }` carries the new `image`, so the UI shows it without reading the page again.
- Wikidata needs an MBID. An artist without one gets a MusicBrainz artist search, used only when exactly one strong match comes back. This raised Wikidata's coverage of the owner's artists from 6 to 26 of 43.

### Imported songs

- Bare songs are resolved in their albums' screen order (`Pictures::rank`), not oldest first.
- Off-screen ones wait rather than being dropped: the UI asks about a song once a session.
- A bare song's album also gets `art` at once (the catalog route). Its cover is then found by name through the steps above, at Last.fm's pace (2 a second) rather than resolution's iTunes budget (about 12 a minute).

## Alternatives

- **Keep two services and make each screen-aware.** That would need the screen state twice and two caches with different rules, and the same album's cover would still be fetched twice (library's `-caa` file and discovery's).
- **Priority inside `Http::pace`.** The pacer would have to know ranks that change as the listener scrolls. Lanes with one worker per paced service leave at most one picture call at any pacer, so a fair first-come gate is enough there.
- **The browser cancels requests instead of a screen report.** Removed `<img>` elements don't reliably cancel their requests, and a cancelled request doesn't stop the work it started on the hub. Covers that are waiting make no request at all.
- **Keep MusicBrainz name search first** (exact release groups). It is 0.5 s and then 3 s from the archive, found nothing Last.fm and iTunes missed, and chose another release group than the known MBID in 2 of 7 cases.
- **TheAudioDB or fanart.tv for artist photos.** TheAudioDB found 70% of the owner's artists (Wikidata 60%), but its free key is a test key, and fanart.tv needs a key the owner doesn't have. They can join `vet` later as one more step, with `lull/THEAUDIODB_API_KEY` or `lull/FANARTTV_API_KEY` in Bitwarden. Not now.

## Consequences

- **`Ctx` gains a field.** Every place that builds a `Ctx` (the hub and five tests) sets it. A branch that adds a new `Ctx` literal needs one line on rebase.
- **Library's outside lookup code goes:** `Lookups`, MusicBrainz name search for covers, and the minute's pause. Its `-caa` cache files are no longer read; those covers are fetched once more.
- **Discovery's `warm` queue for pictures goes.** An answer no longer fetches every picture it names: what is shown is fetched when the screen reports it, at most a quarter of a second later. `lull_core::warm` stays for soulseek and library's local covers.
- **A paced call no longer has a booked time.** Callers that relied on queue order now get first-come order among those still waiting. None relied on more.
- **The fixture hub models the screen route and serves pending pictures in screen order,** so the UI gates can check that the screen the listener stops on fills first.
