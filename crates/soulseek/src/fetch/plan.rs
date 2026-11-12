//! What the better copy's race is for (ADR-0018): the one piece that reads the device's preset.
//! It orders the copies a song may switch to into rounds, a quality band each (`policy::better`,
//! from `StreamProfile::bands`), highest first and never the playing copy's band or lower, one
//! copy per peer (its best), the peers in the order lanes ask them. `race.rs` runs the rounds
//! and knows no settings. Here too is the better copy's fetch: plan, race, then the winner's legs.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;

use tokio::sync::watch;

use super::lanes::Priority;
use super::race::{Raced, Report, Round};
use super::{Fetch, Fetcher, Patience, Skip, Status, Stop, key, reason, stopped};
use crate::budget::Lane as SearchLane;
use crate::policy::{self, Bar};
use crate::rules::Purpose;
use crate::sources::Ranked;

impl Fetcher {
    /// The rounds to race for a song playing `bar.playing`: copies in a higher band that the
    /// preset allows (`policy::better`), highest band first, a round each, one copy per peer.
    /// Not a peer another preset's fetch is transferring, nor one lull cancelled too often today.
    pub(super) fn rounds(&self, f: &Fetch, bar: &Bar) -> Vec<Round> {
        let rules = self.rules_of(f);
        let duration = f.track.duration;
        let held = self.held_elsewhere(f);
        let known = self.sources.ordered(&f.track, &rules, Purpose::Stream);
        let worth = policy::better(&rules.stream, self.sources.preferred(), known, bar, duration);
        let mut peers: HashSet<String> = HashSet::new();
        let mut rounds: Vec<(lull_core::quality::Band, Vec<Ranked>)> = Vec::new();
        for r in worth {
            // Best first, so the first copy of a peer is its best.
            if !peers.insert(r.peer.clone()) || held.iter().any(|h| r.source.is(h)) || !self.lanes.polite(&r.peer) {
                continue;
            }
            let band = policy::band(&r.source, duration);
            match rounds.last_mut() {
                Some((last, copies)) if *last == band => copies.push(r),
                _ => rounds.push((band, vec![r])),
            }
        }
        rounds
            .into_iter()
            .map(|(band, mut copies)| {
                copies.sort_by(lane_order);
                Round {
                    band,
                    candidates: copies.into_iter().map(|r| r.source).collect(),
                }
            })
            .collect()
    }

    /// The better copy of the song playing (ADR-0018): its candidates are raced, and the winner's
    /// running leg carries on here to the end. Nothing of it touches the playing fetch. However
    /// the race ends, it leaves one line in the log (`said`).
    pub(super) async fn better_copy(
        &self,
        f: &Fetch,
        bar: &Bar,
        stop: &mut watch::Receiver<bool>,
    ) -> Result<PathBuf, Stop> {
        let began = Instant::now();
        let mut report = Report::default();
        let result = self.raced_copy(f, bar, stop, &mut report, began).await;
        if !report.said {
            report.took = began.elapsed();
            self.said(f, &mut report);
        }
        result
    }

    async fn raced_copy(
        &self,
        f: &Fetch,
        bar: &Bar,
        stop: &mut watch::Receiver<bool>,
        report: &mut Report,
        began: Instant,
    ) -> Result<PathBuf, Stop> {
        let mut rounds = self.rounds(f, bar);
        if rounds.is_empty() {
            // Nothing known to switch to: one more search, behind the player's and preloads'.
            self.update(f, |p| p.status = Status::Finding);
            tokio::select! {
                r = self.searches.resolve_again_since(&f.track, SearchLane::Prefetch, f.opened) => {
                    r.map_err(|e| Stop::Failed(reason(&e)))?;
                }
                () = stopped(stop) => return Err(Stop::Cancelled),
            }
            rounds = self.rounds(f, bar);
        }
        if rounds.is_empty() {
            self.stays(f, "no better copy turned up");
            return Err(Stop::Failed("no better copy than the one playing turned up".into()));
        }
        let peers = rounds.iter().map(|r| r.candidates.len()).sum();
        self.update(f, |p| {
            p.status = Status::Finding;
            p.peers_found = peers;
        });
        let winner = match self.race_lanes(f, rounds, Priority::Race, stop, report).await? {
            Raced::Won(winner) => *winner,
            Raced::Kept(lane) => {
                self.stays(
                    f,
                    &format!("no better copy sent in time ({}); a queue is kept", report.summary()),
                );
                let kept = self.wait_kept(f, *lane, stop).await;
                report.took = began.elapsed();
                report.winner = kept.as_ref().ok().map(|w| w.source.user.clone());
                self.said(f, report);
                kept?
            }
            Raced::Lost => {
                self.stays(f, &format!("no better copy sent in time ({})", report.summary()));
                self.said(f, report);
                return Err(Stop::Failed("no better copy sent in time".into()));
            }
        };
        if !report.said {
            self.said(f, report);
        }
        f.won.store(true, Ordering::Relaxed);
        let attempt = f.now().attempt + 1;
        let source = winner.source.clone();
        match self
            .legs(
                f,
                attempt,
                &source,
                0.0,
                Patience::Patient,
                Some(winner.started),
                None,
                stop,
            )
            .await
        {
            Ok(path) => Ok(path),
            Err(Ok(Skip::Stalled(why) | Skip::Unavailable(why) | Skip::Slow(why))) => Err(Stop::Failed(why)),
            Err(Err(stop)) => Err(stop),
        }
    }

    /// The playing job says why the song stays on its copy (Activity shows it).
    fn stays(&self, f: &Fetch, why: &str) {
        if let Some(live) = self.playing_in(&key(f)) {
            let note = format!("Stays on this copy: {why}");
            self.update(&live, |p| p.note = Some(note));
        }
    }

    /// The one line a race leaves in the log.
    fn said(&self, f: &Fetch, report: &mut Report) {
        report.said = true;
        tracing::info!(
            track = %f.track.id,
            job = f.job_id,
            bands = ?report.bands,
            lanes = report.asked,
            winner = report.winner.as_deref().unwrap_or("none"),
            secs = report.took.as_secs_f32(),
            cancels = report.cancelled,
            "better-copy race"
        );
    }
}

impl Report {
    /// "asked 6 peers for lossless, high lossy": for a note.
    fn summary(&self) -> String {
        let bands: Vec<String> = self.bands.iter().map(|b| format!("{b:?}")).collect();
        format!("asked {} peers: {}", self.asked, bands.join(", "))
    }
}

/// The order lanes ask peers in: a free slot first, then the speed lull measured, then what the
/// peer reports, which only breaks ties (ADR-0015); a peer that stalled before goes last.
fn lane_order(a: &Ranked, b: &Ranked) -> std::cmp::Ordering {
    let measured = |r: &Ranked| if r.speed_measured { r.speed } else { 0.0 };
    (a.stalled_before, !a.free_slot)
        .cmp(&(b.stalled_before, !b.free_slot))
        .then(measured(b).total_cmp(&measured(a)))
        .then(b.source.upload_speed.cmp(&a.source.upload_speed))
}
