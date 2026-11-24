//! Scores one query: is what the listener wants first, and if not, why.

use std::fmt;

use super::listening::{Listening, WantedArtist, WantedSong};
use super::queries::Target;
use crate::search::merge::{Merged, Score};
use crate::search::personal::snapshot::Snapshot;
use crate::search::query::Query;
use crate::search::query_set::top_line;
use crate::search::{Ranked, Top, sections};

/// How far down a section the reciprocal rank looks.
pub const DEPTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cause {
    /// Another row's words fit the query better than the wanted one's.
    WrongFit,
    /// The wanted row is another take of the song (live, remix...) and the leader is not.
    VersionRule,
    /// Both fit alike and the leader is the better known.
    Popularity,
    /// Another of the listener's own led it, by the short-search rule (`merge::own_leads`).
    OwnFirst,
    /// The listener keeps the wanted one, and the leader they don't: only popularity put it first.
    OwnIgnored,
    /// No source's answer holds it.
    Missing,
    Other,
}

impl Cause {
    pub const ALL: [Cause; 7] = [
        Cause::WrongFit,
        Cause::VersionRule,
        Cause::Popularity,
        Cause::OwnFirst,
        Cause::OwnIgnored,
        Cause::Missing,
        Cause::Other,
    ];
}

impl fmt::Display for Cause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Cause::WrongFit => "wrong fit",
            Cause::VersionRule => "version rule",
            Cause::Popularity => "popularity",
            Cause::OwnFirst => "another of the listener's own led",
            Cause::OwnIgnored => "the listener's own ignored",
            Cause::Missing => "missing from the answers",
            Cause::Other => "other",
        })
    }
}

pub struct Outcome {
    /// The top result is the wanted song or artist.
    pub top1: bool,
    /// Its place (1 first) in its own section, the songs or the artists, when it is there.
    pub rank: Option<usize>,
    /// What led instead, and why, when the top result is not it.
    pub miss: Option<Miss>,
}

pub struct Miss {
    pub cause: Cause,
    /// "kind: artist - title" of the top result.
    pub led: String,
}

impl Outcome {
    /// 1/rank within the first `DEPTH` of its section, else 0.
    pub fn reciprocal_rank(&self) -> f64 {
        self.rank.filter(|&r| r <= DEPTH).map_or(0.0, |r| 1.0 / r as f64)
    }
}

/// A row as the ranking compares it, and whether the listener keeps it.
#[derive(Clone, Copy)]
struct Row {
    score: Score,
    kept: bool,
}

/// The result of ranking `m` for `q`, judged for the wanted target. `personal` is the snapshot
/// the ranking was given, if any: its rows' scores are then the blended ones.
pub fn judge(
    target: Target,
    listening: &Listening,
    q: &Query,
    m: &Merged,
    r: &Ranked,
    personal: Option<&Snapshot>,
) -> Outcome {
    let scores = sections(q, m, personal);
    let rows = Rows {
        listening,
        m,
        r,
        artists: &scores.artists.lifted,
        albums: &scores.albums.lifted,
        songs: &scores.songs.lifted,
        personal,
    };
    match target {
        Target::Song(i) => rows.judge_song(&listening.songs[i]),
        Target::Artist(i) => rows.judge_artist(&listening.artists[i]),
    }
}

/// What the ranking saw: every row's score, beside the rows and their order.
struct Rows<'a> {
    listening: &'a Listening,
    m: &'a Merged,
    r: &'a Ranked,
    artists: &'a [Score],
    albums: &'a [Score],
    songs: &'a [Score],
    personal: Option<&'a Snapshot>,
}

impl Rows<'_> {
    fn judge_song(&self, wanted: &WantedSong) -> Outcome {
        let is = |i: usize| wanted.is(&self.m.songs[i].artist, &self.m.songs[i].title);
        let place = self.r.songs.iter().position(|&i| is(i));
        let top1 = self
            .r
            .top
            .as_ref()
            .is_some_and(|t| t.kind == "track" && self.song_row(t).is_some_and(is));
        let wanted_row = place.map(|p| Row {
            score: self.songs[self.r.songs[p]],
            kept: wanted.kept,
        });
        self.outcome(
            top1,
            place,
            wanted_row,
            self.m.songs.iter().any(|s| wanted.is(&s.artist, &s.title)),
        )
    }

    fn judge_artist(&self, wanted: &WantedArtist) -> Outcome {
        let is = |i: usize| wanted.is(&self.m.artists[i].name);
        let place = self.r.artists.iter().position(|&i| is(i));
        let top1 = self
            .r
            .top
            .as_ref()
            .is_some_and(|t| t.kind == "artist" && self.artist_row(t).is_some_and(is));
        let wanted_row = place.map(|p| Row {
            score: self.artists[self.r.artists[p]],
            kept: wanted.kept,
        });
        // An artist in the answers but not in the ranked list was left out as borrowing a name.
        self.outcome(
            top1,
            place,
            wanted_row,
            self.m.artists.iter().any(|a| wanted.is(&a.name)),
        )
    }

    fn outcome(&self, top1: bool, place: Option<usize>, wanted: Option<Row>, in_answers: bool) -> Outcome {
        let miss = (!top1).then(|| Miss {
            cause: match (wanted, in_answers) {
                (_, false) => Cause::Missing,
                (Some(wanted), true) => self
                    .leader()
                    .map_or(Cause::Other, |lead| why(lead, wanted, self.personal.is_some())),
                (None, true) => Cause::Other,
            },
            led: top_line(self.r, self.m),
        });
        Outcome {
            top1,
            rank: place.map(|p| p + 1),
            miss,
        }
    }

    fn song_row(&self, top: &Top) -> Option<usize> {
        self.m.songs.iter().position(|s| s.id().0 == top.id)
    }

    fn artist_row(&self, top: &Top) -> Option<usize> {
        self.m.artists.iter().position(|a| a.id().0 == top.id)
    }

    /// The top result as a row, when it is a song, an album or an artist.
    fn leader(&self) -> Option<Row> {
        let top = self.r.top.as_ref()?;
        let l = self.listening;
        match top.kind {
            "track" => self.song_row(top).map(|i| Row {
                score: self.songs[i],
                kept: l.keeps_song(&self.m.songs[i].artist, &self.m.songs[i].title),
            }),
            "artist" => self.artist_row(top).map(|i| Row {
                score: self.artists[i],
                kept: l.keeps_artist(&self.m.artists[i].name),
            }),
            "album" => self.m.albums.iter().position(|a| a.id().0 == top.id).map(|i| Row {
                score: self.albums[i],
                kept: false,
            }),
            _ => None,
        }
    }
}

/// Why `lead` is before `wanted`, by the order `merge::rank` sorts in: the short-search rule's
/// mark (`merge::own_leads`), fit, the song itself before
/// its other versions, then popularity in tenths (where the listener's own breaks ties). With
/// `blended` (`merge::rank_blended`) popularity already holds the listener's affinity and decides
/// alone: the leader leads by it, and the listener's own were outweighed when `wanted` is kept
/// and the leader isn't.
fn why(lead: Row, wanted: Row, blended: bool) -> Cause {
    let tenths = |r: Row| (r.score.popularity * 10.0).floor() as i64;
    let own_outweighed = wanted.kept && !lead.kept && (blended || tenths(lead) >= tenths(wanted));
    if lead.score.own_first && !wanted.score.own_first {
        Cause::OwnFirst
    } else if lead.score.fit > wanted.score.fit {
        Cause::WrongFit
    } else if lead.score.fit < wanted.score.fit {
        // The top result went to another kind of row (an album's lean toward the top).
        Cause::Other
    } else if wanted.score.version && !lead.score.version {
        Cause::VersionRule
    } else if lead.score.filler != wanted.score.filler {
        Cause::Other
    } else if own_outweighed {
        Cause::OwnIgnored
    } else if lead.score.popularity > wanted.score.popularity {
        Cause::Popularity
    } else {
        Cause::Other
    }
}
