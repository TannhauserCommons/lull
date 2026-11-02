# ADR-0031: Artist pages merge every catalog, and show only real photos

- Status: accepted
- Date: 2026-11-02

## Context

The listener compared BIJI's artist page with Spotify's. lull's showed two singles and two songs, labelled "by your plays", with the Cyberpunk: Edgerunners 2 album cover as the artist's photo. Spotify showed a photo, the monthly listeners, ten popular songs and the whole discography. Meanwhile lull's own search already found BIJI, Pt. 1 and Pt. 2, Boya, ESMERE, LEGENDS and more under the same artist id.

The cause: the page's discography was MusicBrainz's release groups only, and its top songs ListenBrainz artist radio only. Small and new artists are often missing from both. The photo was whatever Deezer returned for the artist's name, and Deezer returns an album cover for an artist without a photo. Deezer and iTunes also each list two artists called Biji.

## Decision

- **Discography:** merge iTunes's artist lookup (`lookup?id=<artistId>&entity=album` and `entity=song`), Deezer's artist albums where Deezer is open, and MusicBrainz's release groups. Releases are de-duplicated by loose title within a year, and grouped as Spotify does: albums, singles and EPs, compilations, and appears on. iTunes is preferred as a release's identity, because its tracklist and cover come in one call; MusicBrainz adds the MBID.
- **Popular:** Last.fm's `artist.getTopTracks` play counts, placed on the catalog's songs. A song the catalog doesn't list for the artist isn't shown, because Last.fm pools every artist of a name. Without Last.fm, the catalog's order is used (Deezer's rank, then Apple's order).
- **Telling namesakes apart:** MusicBrainz's URL relationships name the iTunes and Deezer artists, fetched in the same call as the tags, so it costs no extra call. Without a link, iTunes's best-known artist of exactly the name is used, and Deezer's only when it is the only one of that name.
- **Photos:** Wikimedia Commons by way of Wikidata (by MBID, one query for the page's artists) first. Otherwise Deezer's picture, but only for a surely-same Deezer artist, and only when a perceptual hash says it is none of the artist's covers. With no way to tell, there is no photo, and each design draws a deliberate fallback.

## Alternatives

- **MusicBrainz alone, with more calls** (secondary types, recordings). Rejected: it is paced at one call a second for all of lull, and still lacks what only stores carry.
- **Apple Music's artist artwork, or Spotify's API.** Rejected: Apple's isn't in the Search API (only on its web pages, which would mean scraping). Spotify's needs an account and a key for each hub.
- **TheAudioDB or fanart.tv.** Rejected: both need a key for each hub, and their free tiers are meant for testing.
- **Trusting Deezer's picture.** Rejected: that is how a cover became BIJI's photo.
- **Comparing Deezer's picture id with album cover ids.** Rejected: where Deezer is closed (the homelab), its album lookups for an artist answer empty, so there is nothing to compare with. The pictures themselves can always be compared.

## Consequences

- A first visit to an artist page costs up to three iTunes calls (from what search leaves spare), two MusicBrainz calls, one Wikidata query, a few Deezer calls, and small picture fetches. Everything is cached, and a section that misses the 4 s answer arrives through `artist.updated`.
- Artists that neither Wikidata nor a vettable Deezer picture covers have no photo (BIJI today). That is deliberate.
