# ADR-0032: lull links are public pages with ids after the `#`

- Status: accepted
- Date: 2026-11-03

## Context

The listener wanted a song, album, artist or playlist shared from their lull to open in a friend's lull, even when the friend runs another hub, and to land on a public page for anyone without lull. lull had two menu items: "Copy link" (song.link or Last.fm, public, but a friend's lull couldn't open it) and "Copy lull link" (this hub's own address, which opens only on the sender's tailnet and gives away its name).

There is no lull service on the internet (PROJECT.md: no hosted relay), and a hub's address must never be in a link.

## Decision

- **One link:** the public page lull already found (song.link, album.link, Last.fm), followed by `#lull?` and what lull knows of the item: kind, artist, title, album, the album's artist, length, MBID, and the album's Deezer or iTunes id. Web pages ignore what follows `#`, and the browser never sends it to them. lull ids are hashes of names, so the same names give the same ids on every hub.
- **Any hub opens it** (`GET /api/open`): the ids after `#lull?` first, else the public page's own ids (song.link and Apple Music's iTunes ids, Deezer's, MusicBrainz release groups, Last.fm's names). So links from those services open in lull too.
- **Ways in:** pasted into search, `#/open?link=` in the web UI, `lull://open?link=` for the desktop app (registered by the installed app only), and Android's Share sheet and `lull://` links.
- **Playlists** share the link they were imported from, which any lull opens as the outside preview. A playlist made in lull lives only on its hub, so it has no link.

## Alternatives

- **A lull-owned "open" page** (for example on GitHub Pages) that tries `lull://` and falls back to song.link. A link clicked in a browser would then reach the app, and Android could verify App Links. Not done yet: it publishes something under the listener's name, which needs their OK. The `#lull?` ids would carry over unchanged.
- **The sender's hub address with a token.** Rejected: it leaks the tailnet name, and only works for people on it.
- **Encoding a playlist's songs in the link.** Rejected for now: there's no public page to land on without lull, and long links break in some chat apps.
- **Claiming song.link's https links on Android.** Rejected: without that domain's verification Android doesn't open them in an app by default, and lull would take every song.link link.

## Consequences

- A link clicked in a browser lands on the public page. Only paste, Share or `lull://` open it in lull, until an open page exists.
- Opening an iTunes id costs one iTunes call, which waits its turn of the shared budget. Opening a Deezer id costs one Deezer call, and a MusicBrainz release group one MusicBrainz call. Opening a lull link costs nothing outside.
