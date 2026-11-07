# ADR-0035: An artist is what is anchored to it, and a name shared by several is a page of them

- Status: accepted
- Date: 2026-11-07 (accepted by the owner the same day: "build it")
- Extends [ADR-0020](0020-library-index-and-saved-ids.md) (aliases), [ADR-0031](0031-artist-pages.md) (telling namesakes apart on artist pages), [ADR-0032](0032-lull-links-are-public-pages-with-ids.md) and [ADR-0034](0034-lull-links-are-cards-served-from-cloudflare.md) (links), [ADR-0100](0100-picks-stable-at-the-core-fresh-at-the-edge.md) (picks).

## In plain words (for the owner)

Several artists can share a name: MusicBrainz knows seven called exactly Nirvana. Today lull makes one artist of them all, so the 1960s British Nirvana can't be reached, and a link to it opens the American one.

**One rule fixes it.** An artist is held together by *anchors*: a service's id for it (MusicBrainz, Deezer, Apple), or a specific album or song of theirs, your own files included. Two things are the same artist when they share an anchor, or when they have the same name and nothing tells them apart. A name alone never decides between two artists.

**Everything that names an artist goes to every artist it could be.** If that's one, you land on its page, as today. If it's several, you land on the name's page, "Artists called Nirvana": each one with its photo, a short description ("60s band from the UK") and an album you'd know. You tap the one you mean. That page is the whole answer to "which one?": there is no separate question box.

**Your likes, follows, hides and plays belong to the exact artist, album or song you made them on: another artist with the same name never gets any of them.**

**What happens to what you already have:**
- **Plays, likes, playlists and downloads** are of songs, and a song stays the same song. Nothing is lost or changed.
- **Which artist a song shows under** follows its album. If lull can't tell which Nirvana an old song is by, it is listed on "Artists called Nirvana" under "songs lull can't place yet", and you can say whose it is. lull never files it under one silently.
- **A follow or hide of "Nirvana"** from before stays on the Nirvana your songs and files point to. Only if they point to none, or to two, does Following show "Nirvana: which one?"; until you pick, it shows on no Nirvana's page. The other Nirvanas start with nothing of yours.
- **Telling lull yourself:** on any album, "This is another Nirvana" or "This is by Nirvana (UK)". Your word is an anchor too, and lull keeps it.

**Your earlier answers are in:** deluxe and remastered editions stay separate albums. A like follows the same recording onto its single, album and compilation, but not onto a live take, remix, edit or re-recording.

It lands switched off, rewrites nothing you have, and switching it off gives back today's lull.

## Context

Verified in the code on main (2026-11-07):
- **Ids are hashes of names.** `ArtistId::of(name)`, `AlbumId::of(artist, album)` and `TrackId::of(artist, album, title)` hash names (`crates/core/src/ids.rs`). Every crate mints them from whatever text it has: catalogue rows, Soulseek tags, Jellyfin, Last.fm history, imports.
- **Readers make the id again from the name.** Taste (`crates/discovery/src/taste.rs`), stats, radio, hiding and following hash the name stored next to a play, like or follow again. Following is refused unless `ArtistId::of(name)` equals the id (`crates/collection/src/follows.rs`). A second artist of a name would fold back into the first in all of them.
- **Ids leave the hub.** Browsers keep song ids in their offline store and player state, the Android app names downloads by them, Subsonic clients cache lull's song, album and artist ids, and M3U exports carry `/api/stream/t-…`. Share links carry names and outside ids, never lull ids.
- **One record per name.** `crates/discovery/src/artist/identity.rs` keeps the exact ids lull holds for an artist (`discovery_artist_ids`), at most one per source, and never compares names. It chooses well which namesake an artist page is about, but there is only room for one; two ids of one source "cancel each other".
- **Search merges namesakes into one row** (`same_artist` in `crates/discovery/src/search/merge.rs` joins by loose name).
- **Links:** ADR-0034's artist link has room for `dz` and `mb`, but the hub writes only `mb` (`crates/discovery/src/share_host.rs`). Opening a link makes `ArtistId::of(name)` and drops the MBID that came with it. Last.fm's links are names only.
- **Same-titled albums overwrite each other.** Two albums with one artist name and title (a band's several self-titled albums) get one `AlbumId`, and `catalog_albums` keeps whichever was written last (`crates/core/src/catalog.rs`).
- **Exact ids lull reads, then drops or mislabels:** a recording's MBID is dropped when a song record is built (`crates/discovery/src/store.rs`, `track`); the library's `Album.musicbrainz_id` holds a release's MBID (`crates/library/src/index.rs`), while discovery's same field holds a release group's. Folder mode and Jellyfin read the album artist's MBID from tags and provider ids.
- **lull already finds anchors for songs that lack them:** a "bare" song is looked up in the catalogues when the UI shows it, and the attempt is remembered (`catalog_resolutions`, `POST /api/catalog/resolve`).
- **A newer schema step under a known owner stops an older build from starting** (`crates/core/src/db.rs`, `migrate`).

Live answers (2026-11-07):
- MusicBrainz's search for "Nirvana" has seven artists named exactly that, scored 60 to 100, each with a comment ("1980s–1990s US grunge band", "60s band from the UK"). The score ranks; it can't decide.
- Deezer and Apple call the British band "Nirvana (UK)", with no disambiguation field. Apple lists six artists named exactly Nirvana, Deezer two. So by name lull also makes two artists of one band.

What others do (checked 2026-11-07; sources at the end):
- **MusicBrainz:** permanent MBIDs; a merged MBID answers under the survivor (Picard compares the id it asked with the id it got); aliases by locale; an album is a release group, and a recording can be on 164 releases.
- **Last.fm** keeps one page per name and says it can't tell namesakes apart; its `mbid` look-ups have returned wrong artists.
- **Navidrome and Jellyfin** still key artists by name, with open namesake bugs and open fixes that keep today's ids as the default; Navidrome's earlier re-key lost artist favourites.
- **Lidarr** keeps every old id and refuses a name that matches two artists; **beets** and **Plex** list candidates for the listener to pick.

## Decision

### The rule

**An artist is a set of anchors. Two things are the same artist when they share an anchor, or when they have the same name and no anchor tells them apart.**

- **An anchor** is an exact id of the artist (MBID, Deezer artist id, Apple artistId), a specific work credited to it (an album or song record, a local file's album), or the listener's word ("this album is by that Nirvana", "this is another Nirvana").
- **Anchors join across names:** Deezer's "Nirvana (UK)" and MusicBrainz's "Nirvana" hold the same MBID through MusicBrainz's links, so they are one artist. Nothing is joined by name likeness.
- **Anchors tell apart:** two different ids from one source, with no anchor shared, are two artists.
- **Names join only what nothing tells apart.** A band with no ids anywhere is one artist, as today. A song credited "Nirvana" with no anchor fits every Nirvana, so it belongs to none of them until an anchor says which.
- **A name is never an anchor.**

The second clause is what keeps an artist with no ids whole. A rule of "same artist only when an anchor is shared" would make one artist per album of every band lull has no id for.

### What the listener did belongs to what they did it on

**A like, follow, hide or play belongs to the exact artist, album or song it was made on. A namesake never inherits any of it.**
- **Likes and plays** are of a song, so they go wherever the song goes (its anchors), and show only there: on that artist's page, in its search row, in its picks and taste. Never on a namesake's.
- **Follows and hides** are of an artist. A new one is stored on the artist's own id.
- **What the listener did before this ADR** stays with the artist it was made with: the one the listener's own songs and files of that name point to. A follow or hide kept under a name's id is anchored by those songs, through the same rule as everything else. When they point to one artist, it is that artist's. When they point to none or to several, it is shown as "which one?" and counts for nobody, on no page and in no taste or picks, until the listener picks.
- **Other namesakes start with nothing:** no likes, plays, follows, hides, taste weight or pick history.
- **A song lull can't place** (several artists fit, no anchor) counts for none of them: its plays and like stay on the song, and the song shows only on the name's page, under "songs lull can't place yet".

### Where anything that names an artist goes

**To every artist it could be.** A link, a search row, a song's credit, a follow, a Last.fm name, a file's tags: its anchors pick the artists that hold them; with none, its name picks every artist of that name.
- **One artist:** its page, as today.
- **Several:** the name's page.
- **None known yet:** a new artist, from the anchors and name the reference brings.

There is no other branch. A link that carries an id, a link that carries an album, a bare name from Last.fm and a local file are all the same call.

**Finding anchors for what has none** uses lull's existing look-up of bare songs (`catalog_resolutions`): a work credited by name only is looked up as a work (its album, its song) in the catalogues when it is first shown, and the catalogue's album brings its artist's id. Most songs get their anchor this way, so most references never reach a name's page.

### The name's page: "Artists called Nirvana"

- **Every artist of the name**, with or without ids: the vetted photo, else the design's own fallback (never an album cover, ADR-0031); MusicBrainz's comment, else type, country and years; one album the listener would know; "yours" on those the listener has songs or a follow of.
- **"Songs lull can't place yet":** the listener's songs credited by the name that no anchor places, each with **Whose is this?**
- **Who is on it:** the artists lull has met by anchors (search, links, files, recommenders), plus MusicBrainz's artists named exactly that, asked when the page opens (paced, cached 30 days).
- **When only one artist has the name,** a reference never shows this page: it goes straight to the artist.
- **A page, not a dialog:** it has an address (`#/artists/named/<name>`), the back button works, and it replaces the chooser and its "None of these" from the first draft.

### The listener's word

- **"This is by Nirvana (UK)"** on an album or a song: the work becomes an anchor of that artist.
- **"This is another Nirvana"**: lull makes an artist from that work, with an anchor of the listener's own (`you:<n>`); the listener's other works can join it the same way. This is how two namesakes that no service knows are told apart. The new artist takes only the likes and plays of the songs moved to it (they are those songs'); follows and hides stay with the artist they were made on.
- **The listener's word beats a service's id** for the works it names (they know their own files), and stays until they change it.
- A follow moved from the name's page is the same kind of word.

### Ids

- **A name's id, `ArtistId::of(name)`, now means the name.** It opens the one artist of that name, or the name's page. Every id already handed out (browser routes, Subsonic clients) keeps opening something right. What the listener kept under a name's id before (follows, hides, pick seeds) is not the name's: it belongs to their artist, as the section above says.
- **An artist's own id** is minted from its name and its first anchor (`mb:<MBID>`, `dz:<id>`, `it:<id>`, `you:<n>`), once, and never renamed. While a name has one artist, that artist answers to the name's id too.
- **Gaining an anchor later** adds it to the set. If the anchor already belongs to another artist (the same band under another spelling, or a MusicBrainz merge, seen when the id asked and the id answered differ), the two become one, and the newer id is kept as an alias of the older (ADR-0020's pattern), resolved in one place.
- **Albums use the same rule.** Two albums of one name and title are told apart by their anchors (release group MBID, catalogue album id): a band's several self-titled albums stay separate albums. Deluxe and remastered editions stay separate albums, as today (owner, 2026-11-07).
- **Songs:** a song's id stays per release (it names a file and a tracklist). "The same recording" is its MBID; else an ISRC shared with the same artist (a hint: one ISRC can name several catalogue tracks); else lull's same-take rule (loose title and artist, durations close; `same_take`), never across version words (live, remix, edit, re-recording).
- **Exact ids lull reads are kept:** a song's recording MBID and ISRCs go on its record (an additive field); the library's album MBID is labelled a release, with its release group beside it.

### Links

- **Every link carries at least one anchor.** Song and album links carry their work (names and album ids, as today). An artist link carries every id the hub holds (`mb`; new: `dz`, already in ADR-0034's format, and `it`, Apple's artistId), and always one album of theirs (`al`), so even an artist with no ids is opened by its work.
- **`ds`**, the short description, lets the card and the page say which one it is. Both are optional keys of version 1 (unknown keys are ignored, ADR-0034).
- **Opening** is the rule above: the link's anchors, else its name. Every link already copied still opens.
- **The Worker** keeps its rule (a picture by id, else only when one artist has the name) and shows `ds` under the name. Using `al` for the picture is the share session's call, within the Worker's limits.

### Search, library and Subsonic

- **Search:** rows that anchors tell apart are separate rows, ranked as today. A row gets the description line only when another artist shares its name, so nobody else pays for it.
- **Library:** MBIDs in tags (`MUSICBRAINZ_ALBUMARTISTID`, `MUSICBRAINZ_ARTISTID`; a tag with several ids counts only when it names one per artist, Jellyfin's rule) and Jellyfin's provider ids are anchors; each saved album is one too. Two "Nirvana" folders with different MBIDs are two artists.
- **Subsonic:** each artist has its own id and carries its MBID in OpenSubsonic's `musicBrainzId`; names are unchanged ("Nirvana (UK)" would leak into scrobbles). A client holding a name's id from before gets that name's albums, as today.

### Taste, picks and likes

- **Every artist weight goes to the song's artist under the rule,** read through the song's id. Taste, stats, radio, hiding and following stop hashing names; one function in the catalogue answers "whose song is this" for all of them, and following accepts an artist's own id.
- **Nothing the listener did on one artist reaches a namesake:** follows, likes, plays, hides, "more like this" and "less like this" weigh only the artist they were made on (see "What the listener did belongs to what they did it on").
- **A song that fits several artists** gives no artist weight until it is placed; its tags still count. Last.fm's top artists, which are names, are the same: one artist of the name gets the weight; several share nothing.
- **Recommenders' ids** (ListenBrainz's MBIDs, Deezer's ids) are anchors and land on the right artist.
- **Picks:** "no artist they already like" is by artist, not name; a namesake of a liked artist may be picked, and shows its line. "Songs they know" is by recording, not by loose artist and title (`known_songs` today), so a namesake's song of the same title isn't taken for one the listener knows.
- **Likes follow the recording** onto its other releases: the heart shows on the single, the album and the compilation. Not on another version (live, remix, edit, re-recording). The like is still stored once, on the song that was liked.
- **Last.fm scrobbles** keep going by name. Last.fm merges namesakes on its side; lull doesn't take that back.

### Migrating every listener's database

- **No existing id changes and no existing row is rewritten.** Plays, likes, playlists, downloads and Subsonic stars are of songs, kept by song id.
- **New tables under a new migrations owner, `identity`** (in `crates/core`, read by every crate): artists and their anchors (with where each came from), placements of works, aliases, and the listener's words. An older lull doesn't know that owner, so it still starts after a downgrade.
- **An overlay, not a rewrite:** the catalogue's one reader of a song or album applies it; stored JSON keeps the name's id. With the flag off, the overlay isn't read, so lull is today's.
- **Carried over:** `discovery_artist_ids` holds the exact ids behind each artist today; each row becomes an anchor of that artist.
- **Tests that nothing carries over to a namesake** (the owner's rule), each on a hub with two Nirvanas told apart by MBIDs and with the flag on:
  - a like, play, follow, hide, "more like this" and "less like this" on one shows nowhere on the other: not its page, its search row, its picks, its radio, its stats or its taste weight;
  - from before: the listener's songs and a follow of "Nirvana" made when lull knew one, then the second Nirvana appears: the follow, hide and plays stay on the first, and the second starts at zero;
  - from before, with the listener's songs split between the two, or with none: the follow is "which one?" and counts for neither until picked; after the pick, only the picked one has it;
  - a song that fits both: its like and plays count for neither, and it shows only on the name's page;
  - one band under two spellings joined by an MBID: its likes and follows are the joined artist's, once each.
- **Tests on real-shaped data:** a synthetic hub database shaped like a real one (thousands of songs; namesakes with and without ids; a band's several self-titled albums; one band under two spellings; Last.fm history by name; follows, playlists and offline lists), checking that every play, like, playlist item, follow and download opens the same song before and after; that off, the API answers are byte-identical to today's; and that on then off gives back the same.

### Performance and the start-up budgets

- **Hot paths stay a hash.** For a name with one artist (nearly every name), placing a song costs one in-memory look-up. Names with several artists are loaded at start in one small read.
- **No outside call at start.** Outside calls happen only on need: the existing bare-song look-up when a song is shown, and one MusicBrainz name search (1 a second, shared pace, cached 30 days) when a name's page opens.
- **The web UI:** the name's page is its own file, loaded only when one is opened (as `settings-fit.js` is). Every route pays only the description line and the album menu's two items, inside the +1,000 B rule; the landing commit gives the measured bytes. No budget raise is expected; if one is needed, it is asked for with its reason (quality.md).
- **The Worker** makes no extra outside call: `al` and `ds` are words in the link.

### How it lands

Behind a flag, `namesakes` (ADR-0096), in pieces that each land through the queue with their tests:
0. standalone fixes first: keep recording MBIDs and ISRCs on songs; label the library's album MBID a release, with its release group;
1. core: the `identity` tables, the overlay, artists' own ids, "whose song is this";
2. discovery: anchors from every source, search rows kept apart, the description line;
3. discovery: opening by the rule, the name's page data, the listener's word;
4. web: the name's page, "songs lull can't place yet", the album menu's items, likes on the same recording; looked at in the five designs, day and night (`themes:check`);
5. links: anchors at share time and the new keys; the Worker's part with the share session;
6. library and Subsonic: tag MBIDs and Jellyfin's ids as anchors, `musicBrainzId`;
7. taste, picks, stats, radio, hiding and follows by artist;
8. albums: same-titled albums kept apart;
9. the migration suite; then the owner tries it, and the flag is removed.

## The rule against every case

| Case | Anchors | Where it goes |
|---|---|---|
| A band with no ids anywhere, five albums | none | one artist: same name, nothing tells the albums apart |
| Two real bands, no ids anywhere | none | one artist, as lull can't know; the listener's "This is another Nirvana" splits them |
| US Nirvana (MBID A), UK Nirvana (MBID B) | different MBIDs | two artists |
| A song credited "Nirvana", album found in a catalogue | the catalogue album's artist id | that artist |
| A song credited "Nirvana", found nowhere, several Nirvanas | none | "songs lull can't place yet" on the name's page |
| The same song, only one Nirvana known | none | that Nirvana: nothing tells it apart |
| Deezer's "Nirvana (UK)" and MusicBrainz's "Nirvana" | MusicBrainz's link to one MBID | one artist |
| Local files tagged with an album artist MBID | the MBID | that artist |
| Local files, no tags, saved album | the album | the artist holding it, else by name |
| A lull link with ids | the ids | the artist holding them |
| A lull link, artist with no ids | the album in `al` | the artist holding that album |
| An old link, or a Last.fm link (name only) | none | one artist, or the name's page |
| Last.fm top artist "Nirvana" | none | the weight goes to the one Nirvana, or to none |
| A follow of "Nirvana" from before, the listener's songs all of one Nirvana | those songs' anchors | that Nirvana; the others get nothing |
| A follow of "Nirvana" from before, the listener's songs of two Nirvanas, or none | none that decide | "Nirvana: which one?", counting for nobody until the pick |
| A like on a US Nirvana song | the song | the US Nirvana only; the UK one's page, search row, picks and taste never show it |
| A ListenBrainz or Deezer recommendation | its id | the artist holding it |
| MusicBrainz merges two MBIDs | old and new MBID | one artist, the old id an alias |
| Weezer's several albums called "Weezer" | their release groups | separate albums |
| "Nevermind" and "Nevermind (Deluxe)" | different titles and releases | separate albums (owner's choice) |
| "Lithium" on the single and the album | one recording | the like shows on both |
| "Lithium (Live)" | another recording | its own like |

## Alternatives

- **An ordered list of checks and a chooser dialog** (this ADR's first draft: ids, then context, then the listener's songs, then an earlier choice, else "Which Nirvana?"). Rejected by the owner (2026-11-07): special cases where one rule does.
- **Same artist only when an anchor is shared.** Rejected: every band lull has no id for would split into one artist per album.
- **Re-key everything by MBID** (`ar-mb-<MBID>`). Rejected: Navidrome's re-key lost listeners' favourites; an artist that gains an id would change id, breaking every device, client and link that holds it; things with no id still need keys.
- **Pick the best-known namesake when unsure.** Rejected: a silent guess.
- **Put the difference in the name** ("Nirvana (UK)", Discogs's "Nirvana (2)"). Rejected: it changes what goes to Last.fm, tags and Subsonic clients.
- **Join spellings by likeness.** Rejected: that is how namesakes merged in the first place.
- **Rewrite rows to new ids in a migration.** Rejected: it can't be undone, and an older build refuses a newer schema.

## Consequences

- A name that two artists share costs one MusicBrainz search when its page opens, cached for 30 days.
- Two bands that no service knows, with no tags, stay one artist until the listener tells them apart: lull has nothing else to go by.
- Two catalogue profiles of one artist with no MBID are two artists, until an anchor is shared or the listener joins them.
- `docs/context/discovery.md` (artist identity, search, opening links), `share.md`, `api.md` and `taste.md` change with the pieces that change them.

## Sources

- MusicBrainz: [identifiers and merges](https://musicbrainz.org/doc/MusicBrainz_Identifier), [disambiguation comments](https://musicbrainz.org/doc/Disambiguation_Comment), [aliases](https://musicbrainz.org/doc/Aliases), [release groups](https://musicbrainz.org/doc/Release_Group), [recordings](https://musicbrainz.org/doc/Recording), [search](https://musicbrainz.org/doc/MusicBrainz_API/Search), [rate limiting](https://musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting); Picard's handling of a merged id ([album.py](https://github.com/metabrainz/picard/blob/master/picard/album.py)) and its [tag mapping](https://github.com/metabrainz/picard-docs/blob/master/appendices/tag_mapping.rst).
- Last.fm: [same-named artists](https://support.last.fm/t/there-s-another-artist-or-band-with-the-same-name/195); `mbid` look-ups returning wrong artists ([Navidrome #1091](https://github.com/navidrome/navidrome/issues/1091)).
- Navidrome: [0.55.0](https://github.com/navidrome/navidrome/releases/tag/v0.55.0), [persistent ids](https://www.navidrome.org/docs/usage/configuration/persistent-ids/), [#3964](https://github.com/navidrome/navidrome/issues/3964), [PR #5527](https://github.com/navidrome/navidrome/pull/5527).
- Jellyfin: [#9623](https://github.com/jellyfin/jellyfin/issues/9623), [PR #17608](https://github.com/jellyfin/jellyfin/pull/17608).
- Lidarr: [old ids and name look-up](https://github.com/Lidarr/Lidarr/blob/develop/src/NzbDrone.Core/Music/Repositories/ArtistRepository.cs); beets: [candidate display](https://github.com/beetbox/beets/blob/master/beets/config_default.yaml).
- Spotify: [music mixed up with another artist](https://support.spotify.com/us/artists/article/music-mixed-up-with-another-artist/); Apple: [artist pages](https://itunespartner.apple.com/music/support/5229-artist-pages).
