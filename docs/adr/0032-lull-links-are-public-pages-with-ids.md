# ADR-0032: lull links are lull's open page, with ids after the `#`

- Status: accepted
- Date: 2026-11-03 (the open page accepted by the listener the same day)

## Context

The listener wanted a song, album, artist or playlist shared from their lull to open in a friend's lull, even when the friend runs another hub, and to land on a public page for anyone without lull. lull had two menu items: "Copy link" (song.link or Last.fm, public, but a friend's lull couldn't open it) and "Copy lull link" (this hub's own address, which opens only on the sender's tailnet and gives away its name).

There is no lull service on the internet (PROJECT.md: no hosted relay), and a hub's address must never be in a link.

## Decision

- **One link:** lull's open page, `https://tannhausercommons.github.io/lull/open`, followed by `#lull?` and what lull knows of the item: kind, artist, title, album, the album's artist, length, MBID, the album's Deezer or iTunes id, and its public page (`page=`: song.link, album.link or Last.fm). Browsers never send what follows `#` to a server, so GitHub sees only that someone opened the page. lull ids are hashes of names, so the same names give the same ids on every hub.
- **The open page** (`site/`, one static page on GitHub Pages, deployed by `.github/workflows/pages.yml`) makes no request of its own: no tracking, no outside scripts or fonts. On a click it:
  - opens the listener's lull in the browser when they have told it the address (kept in that browser's storage only), with "use a different lull";
  - else the app: on Android, **Open in the lull app** is an intent URL (`intent://open?link=…#Intent;scheme=lull;package=io.github.tannhausercommons.lull;S.browser_fallback_url=…;end`), which Chrome opens on one tap, or comes back to the page (with `?app=none`, which says the app isn't on the phone and leads with **Get lull**) when it isn't installed; elsewhere it tries `lull://open?link=…` once on load (the desktop app asks "Open lull?"). It also offers "Open in lull in the browser" (which asks for the address once);
  - and always offers the public page (only song.link, album.link and Last.fm are linked, so a link can't send anyone elsewhere) and "Get lull".
- **Android** opens the page's links straight in the app (App Links): an `autoVerify` intent filter for `/lull/open`, verified by `https://tannhausercommons.github.io/.well-known/assetlinks.json` (`packaging/android/assetlinks.json`, the release key's fingerprint).
- **Any hub opens a link** (`GET /api/open`): the ids after `#lull?` first, whatever page precedes them (links copied before the open page had the public page itself in front), else the public page's own ids (song.link and Apple Music's iTunes ids, Deezer's, MusicBrainz release groups, Last.fm's names). So links from those services open in lull too.
- **Other ways in:** pasted into search, `#/open?link=` in the web UI, `lull://open?link=` for the desktop app (registered by the installed app only), and Android's Share sheet.
- **Playlists** share the link they were imported from, which any lull opens as the outside preview. A playlist made in lull lives only on its hub, so it has no link.

## Alternatives

- **The public page itself in front of the ids** (how links were first built, the same day). Rejected once the listener agreed to publish the open page: a click landed on song.link, and only paste or Share reached lull.
- **The sender's hub address with a token.** Rejected: it leaks the tailnet name, and only works for people on it.
- **Encoding a playlist's songs in the link.** Rejected for now: long links break in some chat apps.
- **Claiming song.link's https links on Android.** Rejected: lull can't verify another site's domain, and it would take every song.link link.

## Consequences

- **The open page must stay at its address:** every link copied names it. `OPEN_PAGE` in `crates/discovery/src/open.rs` and `site/` move together, if ever.
- **App Links need the root site:** Android reads `assetlinks.json` from the host's root, which a project site (`/lull/`) can't serve. The `TannhauserCommons/tannhausercommons.github.io` repository serves it, from Pages with a `.nojekyll`. Until it does, Android shows the page, and its **Open in the lull app** button opens the app (below).
- **Android needs a tap:** Chrome on Android opens no app from a page without the listener's gesture, so the page tried `lull://` on load in vain, and a plain `lull://` link isn't offered to the app the same way. The button's intent URL names the app's package and a page to come back to, so one tap opens the song in lull, or tells the listener the app isn't installed; the page tries nothing on load there. The app's `lull://open` intent filter (VIEW, BROWSABLE) takes it as it takes any `lull://open` link, and reads `link` the same way. `site/open.js` builds the URL (`tests/web/open-page.test.mjs` reads it back as Chrome and the app do). A real phone's check is the owner's.
- **Costs:** opening an iTunes id costs one iTunes call, which waits its turn of the shared budget. A Deezer id costs one Deezer call, and a MusicBrainz release group one MusicBrainz call. Opening a lull link costs nothing outside.
