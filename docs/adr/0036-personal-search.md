# ADR-0036: Search leads with the listener's own music, within equally good matches

- Status: accepted (behind the `personal-search` switch, ADR-0096)
- Date: 2026-11-20

## Context

Search ranked a row by how well its words fit, then popularity in tenths, and only then whether it was the listener's (saved, liked, a followed artist) as a yes or no. So the listener's music broke a tie inside one tenth of popularity and nowhere else. The library half was a substring scan of saved songs: a song they played or liked but never saved was found only if a catalog listed it, and "ra" found their Radiohead only once a catalog did. A followed artist was matched by its name's id, the name borrowing ADR-0035 forbids. The taste model already weighs plays, skips, likes, saves and follows, but search never read it.

## Decision

- **A good match always wins.** Fit stays the first key, then the song before its other versions, then the catalog's rows before a gap-filler's. The listener's music only orders rows of equal fit.
- **Among equal matches one blended value:** `popularity + W × affinity`, with `W = 0.5` (`merge::AFFINITY_WEIGHT`), chosen by the personal case set and the query set (discovery.md#personal-search). The top result's **kind** (artist, album or song) is chosen from the catalogs' scores before the lift and the row within the kind by the blend: the lift reaches a song through its own records and its artist's score at half, but an artist's row only through ids, so with the lift in the choice a listener's own album beat the band's name for the band's name (eval: artist-name top-1 40% to 12%; case `the_artist_leads_its_own_name…`).
- **Affinity is one number from 0 to 1** per song, album and artist, a pure function of taste's signals (`search/personal/score.rs`; the formula is in taste.md#search-affinity). It reuses taste's weights and fade, never goes below 0 (search never sinks a row the words match), and is explainable from what the listener did.
- **Taste in tiers:** a song's own signals are the strongest (plays, a like, saved, in a playlist); an artist's weight counts for half, reached **only through ids** (a row's catalog artist id against the ids the listener's own songs gave, never a name, so a namesake gets nothing); genres and similar artists are not used (no call per search). A genre tier was tried on the owner's listening (a song's score gaining 0.2 of its best genre's standing in taste): top-1 did not move (25.1% both) and MRR@10 rose 0.364 to 0.367 while three queries fell out of the answers, so it was dropped. Rows from a catalog reach a genre only through an artist id, which the eval's export lacks, so only the listener's own songs could be lifted.
- **A take the listener has is the song itself.** A version they play ("Creep (Acoustic)") isn't ranked behind other songs as "another version".
- **An index of their music in memory** (`search/personal/`): saved songs, and songs played, liked or in a playlist, with album, year and genre, matched by the same `Query` as every row. A year or genre finds their songs, ranked below name matches. It replaces the library half as a fast source. It is built when lull starts and rebuilt after a bus event that changes what it holds (a play ended, a like, a follow, a playlist, a save, the library switched, Last.fm's history, a taste reset, a hidden artist, the library's index changing), several in a row coalescing; a search reads the snapshot and calls no database.
- **Off means off:** with the switch off no snapshot exists, nothing is built or read, and search is as it was. Until the first build finishes, a search with the switch on goes as it does off.

## Rejected

- **A boost inside the old popularity tenths:** the listener's music still decided only ties.
- **Lifting by name** (a followed or liked artist's name matched against a row's): a namesake would borrow the lift (ADR-0035). Ids the search rows or a recommender seeded are left out too: `seed_from_search` files a row's id under the artist its name hashes to, so a namesake's id may sit on the listener's artist.
- **Scoring on each search** (`taste()` rebuilds from the database on every call): it costs a database read per search, and rows arriving later would each repeat it.
- **A penalty for what the listener skipped or hid:** search answers what was asked for; a skip only lowers a song's own lift, to nothing.
- **Artist rows in the index:** an index row of a name would merge with a namesake's catalog row and lend it the listener's ids.

## Consequences

- A listener who follows a small artist sees it before a better known namesake only when its ids are held through their own songs, links or ISRC credits; a follow with none of those lifts nothing yet.
- Songs the listener played that the catalog has no record of aren't in the index (the record is where its album and year are kept).
- Changes to the library made outside lull (files added to the music folder, songs added in Jellyfin) reach the index through library's `library.index` event, one per refresh that changed something; Jellyfin is read in full every so often, so a change there waits for that read.
- The weight is a product choice bounded by cases (discovery.md#personal-search); the eval on real listening may move it.
