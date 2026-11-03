//! Search's Playlists section: playlists people made on YouTube and Deezer for what was searched
//! (docs/context/discovery.md#playlists-in-search).
//!
//! - **Relevance first:** how well the title fits the words typed (`Query`, as album titles
//!   fit; a title that is just the words counts as one holding every word, since anyone can
//!   name a playlist anything), then whether it says it is a soundtrack or the songs of
//!   something ("OST", "songs from", "list of songs"). A playlist that doesn't hold most of the
//!   words is left out.
//! - **Then size and popularity:** the site's own order (relevance and plays as the site sees
//!   them), the number of songs, and Deezer's fans. YouTube's search gives neither songs nor
//!   views, so its order stands in.
//! - **Top result:** a query names a film, show or game when two or more playlists that hold
//!   every word say soundtrack. Then the soundtrack album that holds every word takes the top
//!   ("cyberpunk edgerunners 2"), and without one, the best such playlist ("person of interest").

use lull_core::ids::normalize;

use super::merge::{AlbumHit, Fit, PlaylistHit, Query};

/// Playlists listed in an answer.
pub const LISTED: usize = 12;
/// Words that make a title a soundtrack's ("OST", "score").
const SOUNDTRACK_WORDS: &[&str] = &["soundtrack", "soundtracks", "ost", "osts", "score", "bgm"];
/// Phrases that make a title the songs of something ("songs used in the series").
const SONGS_OF: &[&str] = &[
    "songs from",
    "music from",
    "songs used",
    "music used",
    "songs in",
    "songs of",
    "music of",
    "all songs",
    "list of songs",
    "track list",
    "tracklist",
    "original music",
];
/// Fans at which Deezer's following counts fully (log scale).
const FULL_FANS: f64 = 100_000.0;
/// Songs at which a playlist's size counts fully.
const FULL_SIZE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaylistScore {
    pub fit: Fit,
    pub soundtrack: bool,
    /// 0..1: the site's order, size and following.
    pub popularity: f64,
}

/// Whether a title says it is a soundtrack or a list of a work's songs.
pub fn says_soundtrack(title: &str) -> bool {
    let n = normalize(title);
    n.split(' ').any(|w| SOUNDTRACK_WORDS.contains(&w)) || SONGS_OF.iter().any(|p| n.contains(p))
}

pub fn score(q: &Query, p: &PlaylistHit) -> PlaylistScore {
    let fit = q.album(&AlbumHit {
        title: p.title.clone(),
        ..AlbumHit::default()
    });
    let order = 1.0 / (1.0 + f64::from(p.rank) / 3.0);
    let size = p
        .tracks
        .map_or(0.4, |t| (f64::from(t.min(FULL_SIZE)) / f64::from(FULL_SIZE)).sqrt());
    let following = p
        .fans
        .map_or(0.3, |f| ((f as f64 + 1.0).log10() / FULL_FANS.log10()).min(1.0));
    PlaylistScore {
        fit,
        soundtrack: says_soundtrack(&p.title),
        popularity: 0.4 * order + 0.3 * size + 0.3 * following,
    }
}

/// Positions of the playlists worth listing, best first.
pub fn rank(q: &Query, playlists: &[PlaylistHit]) -> Vec<usize> {
    let scores: Vec<PlaylistScore> = playlists.iter().map(|p| score(q, p)).collect();
    let mut order: Vec<usize> = (0..playlists.len()).filter(|&i| scores[i].fit >= Fit::Most).collect();
    let key = |i: usize| {
        let s = scores[i];
        (
            s.fit.min(Fit::Words),
            s.soundtrack,
            (s.popularity * 10.0).floor() as i64,
        )
    };
    order.sort_by(|&a, &b| {
        key(b)
            .cmp(&key(a))
            .then_with(|| scores[b].popularity.total_cmp(&scores[a].popularity))
    });
    order
}

/// What the top result is when the query names a work: its soundtrack album (an index into
/// `albums`, best first), else its best soundtrack playlist (an index into `playlists`).
/// `None` when the query doesn't name a work, and search's own top stands.
#[derive(Debug, PartialEq, Eq)]
pub enum WorkTop {
    Album(usize),
    Playlist(usize),
}

pub fn work_top(q: &Query, albums: &[(usize, &AlbumHit)], playlists: &[PlaylistHit]) -> Option<WorkTop> {
    let fits_as_soundtrack = |p: &PlaylistHit| says_soundtrack(&p.title) && score(q, p).fit >= Fit::Words;
    if playlists.iter().filter(|p| fits_as_soundtrack(p)).count() < 2 {
        return None;
    }
    if let Some((i, _)) = albums
        .iter()
        .find(|(_, a)| says_soundtrack(&a.title) && q.album(a) >= Fit::Words)
    {
        return Some(WorkTop::Album(*i));
    }
    rank(q, playlists)
        .into_iter()
        .find(|&i| fits_as_soundtrack(&playlists[i]))
        .map(WorkTop::Playlist)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playlist(site: &str, title: &str, rank: u32, tracks: Option<u32>, fans: Option<u64>) -> PlaylistHit {
        PlaylistHit {
            site: site.into(),
            id: format!("{site}{rank}"),
            title: title.into(),
            owner: "someone".into(),
            tracks,
            fans,
            rank,
            ..PlaylistHit::default()
        }
    }

    fn album(title: &str) -> AlbumHit {
        AlbumHit {
            title: title.into(),
            artist: "Ramin Djawadi".into(),
            ..AlbumHit::default()
        }
    }

    #[test]
    fn soundtrack_wording_is_recognised_in_its_usual_forms() {
        for title in [
            "Person of Interest Complete Soundtrack OST",
            "Person of Interest : List of Songs",
            "Songs used in Person of Interest",
            "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
            "Interstellar Score",
        ] {
            assert!(says_soundtrack(title), "{title}");
        }
        for title in ["Person of Interest", "Ghost Town", "Hostess Songs", "Postcards"] {
            assert!(!says_soundtrack(title), "{title}");
        }
    }

    #[test]
    fn soundtracks_rank_first_among_playlists_holding_the_words_then_size() {
        let q = Query::new("person of interest");
        let hits = [
            playlist("deezer", "Best of 2014", 0, Some(100), Some(90_000)),
            playlist("deezer", "person of interest", 1, Some(20), None),
            playlist("youtube", "Person of Interest : List of Songs", 2, None, None),
            playlist(
                "deezer",
                "Person of Interest Complete Soundtrack OST",
                3,
                Some(94),
                Some(500),
            ),
        ];
        let order = rank(&q, &hits);
        assert!(
            !order.contains(&0),
            "a playlist that holds none of the words is left out"
        );
        assert_eq!(
            order,
            [3, 2, 1],
            "the big soundtrack, the list of songs, then the bare title"
        );
    }

    #[test]
    fn a_work_with_a_soundtrack_album_has_it_on_top() {
        let q = Query::new("cyberpunk edgerunners 2");
        let lists = [
            playlist(
                "youtube",
                "Cyberpunk: Edgerunners 2 (Original Series Soundtrack)",
                0,
                None,
                None,
            ),
            playlist("youtube", "Cyberpunk: Edgerunners 2 OST Playlist", 1, None, None),
        ];
        let ost = album("Cyberpunk: Edgerunners 2 (Original Series Soundtrack)");
        let other = album("Edgerunners");
        assert_eq!(work_top(&q, &[(4, &other), (7, &ost)], &lists), Some(WorkTop::Album(7)));
    }

    #[test]
    fn a_work_without_one_has_its_best_soundtrack_playlist_on_top() {
        let q = Query::new("person of interest");
        let lists = [
            playlist("deezer", "person of interest", 0, Some(67), None),
            playlist(
                "deezer",
                "Person of Interest Complete Soundtrack OST",
                1,
                Some(94),
                Some(500),
            ),
            playlist("youtube", "Person of Interest : List of Songs", 0, None, None),
        ];
        assert_eq!(work_top(&q, &[], &lists), Some(WorkTop::Playlist(1)));
    }

    #[test]
    fn an_artist_query_keeps_searchs_own_top() {
        let q = Query::new("drake");
        let lists = [
            playlist("deezer", "Drake Best Songs", 0, Some(80), Some(10_000)),
            playlist("youtube", "Drake Mix", 0, None, None),
        ];
        assert_eq!(work_top(&q, &[], &lists), None);
    }
}
