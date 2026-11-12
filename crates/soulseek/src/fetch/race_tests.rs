//! Acceptance tests for ADR-0018: the builder must not edit this file.
//!
//! The better-copy race against the stand-in slskd (`fake_slskd`), with many peers that behave
//! as real ones did (docs/adr/0018, the findings it cites): silent, queued then sending, slow,
//! bursting then crawling, fast. Each test plays a song that starts on a lesser copy (a peer's
//! MP3 128, below the 192 minimum, so the upgrade searches) and watches what the race asks of
//! the peers (the stand-in's request log) and what the player is offered (`stream` events).
//!
//! Real time: tokio's paused clock auto-advances while lull waits on the stand-in's sockets, so
//! every timeout would fire at once. Bounds are generous; the whole file runs in about 30 s.
//!
//! Sizes: every candidate copy is 2 MB for a 200 s song, so it plays at 10 KB/s and the gate
//! (`OUTRUN` 1.5) is 15 KB/s. `FAST` sends 200 KB/s, `SLOW` 5 KB/s.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use lull_core::events::Bus;
use lull_core::ids::{AlbumId, ArtistId, TrackId};
use lull_core::model::{SourceRef, Track};
use lull_core::quality::StreamProfile;
use lull_core::seams::Youtube;
use lull_core::status::Status as Services;
use lull_core::youtube::{Found, Want};
use lull_core::{Error, Result};
use serde_json::Value;
use tokio::time::Instant;

use super::{Fetch, Fetcher, Status};
use crate::cache::Cache;
use crate::fake_slskd::{Answer, Behaviour, Fake, Peer};
use crate::peers::Peers;
use crate::rules::{Current, Rules};
use crate::search::Searches;
use crate::sources::{Source, Sources};

const SIZE: usize = 2_000_000;
const LOW_SIZE: usize = 400_000;
/// Bytes a 20 ms tick: 200 KB/s, well over the gate.
const FAST: usize = 4_096;
/// 5 KB/s: under what the song plays.
const SLOW: usize = 100;
/// What a bursting peer sends at once before it crawls at `SLOW`.
const BURST: usize = 1_000_000;
/// 800 KB/s: a copy all here in 2.5 s.
const FASTEST: usize = 16_384;
const SECS: fn(u64) -> Duration = Duration::from_secs;

struct NoYoutube;

#[async_trait]
impl Youtube for NoYoutube {
    async fn find(&self, _: &Track, _: Want) -> Result<Option<Found>> {
        Ok(None)
    }
    async fn download(&self, _: &SourceRef, _: &std::path::Path) -> Result<()> {
        Err(Error::unavailable("youtube", "not in tests"))
    }
}

fn track(title: &str) -> Track {
    Track {
        id: TrackId(format!("t-{title}")),
        title: title.into(),
        artist_id: ArtistId("ar-1".into()),
        artist: "Artist".into(),
        album_id: AlbumId("al-1".into()),
        album: "Album".into(),
        year: None,
        no: Some(1),
        disc: None,
        duration: Some(200),
        tags: Vec::new(),
        saved: false,
        file: None,
    }
}

fn mp3(title: &str) -> String {
    format!("Music\\Artist\\Album\\01 - {title}.mp3")
}

fn flac(title: &str) -> String {
    format!("Music\\Artist\\Album\\01 - {title}.flac")
}

fn low_path(title: &str) -> String {
    format!("Music\\Rips\\Album\\01 - {title}.mp3")
}

/// A peer's own bytes for a path: different peers' files differ, as they do.
fn bytes_of(user: &str, path: &str, size: usize) -> Vec<u8> {
    let seed = user
        .bytes()
        .chain(path.bytes())
        .fold(7u8, |a, b| a.wrapping_mul(31).wrapping_add(b));
    (0..size).map(|i| seed.wrapping_add((i % 251) as u8)).collect()
}

/// A candidate copy's size: `SIZE`, but another rip's (a peer named "rip...") is a little larger,
/// so it isn't the same file as the others' (no side leg can take its place, ADR-0014).
fn size_of(user: &str) -> usize {
    if user.starts_with("rip") { SIZE + 100_000 } else { SIZE }
}

fn the(user: &str, path: &str) -> SourceRef {
    SourceRef {
        user: user.into(),
        path: path.into(),
        size: SIZE as u64,
    }
}

/// A peer answering with `files` (each a 320 MP3 or a FLAC, by its path), behaving `b`, its
/// answer reporting `mbps` MB/s (the order ranking gives unmeasured peers).
fn peer(user: &str, b: Behaviour, mbps: f64, files: &[String]) -> Answer {
    let files: Vec<(&str, Option<u32>)> = files
        .iter()
        .map(|p| (p.as_str(), (!p.ends_with(".flac")).then_some(320)))
        .collect();
    Answer::new(user, b, &files).says((mbps * 1_000_000.0) as u64)
}

struct Hub {
    fake: Fake,
    fetcher: Fetcher,
}

impl Hub {
    fn profile(&self) -> StreamProfile {
        self.fetcher.rules.get().stream
    }

    /// When the race began: the first request to any of `users`.
    fn began(&self, users: &[&str]) -> Option<Instant> {
        self.fake
            .requests()
            .into_iter()
            .filter(|r| users.contains(&r.user.as_str()))
            .map(|r| r.at)
            .min()
    }

    fn asked(&self, user: &str) -> usize {
        self.fake.requests_of(user).len()
    }
}

/// A hub whose slskd answers searches with `answers`; each title also has the lesser copy
/// ("low", MP3 128, below the 192 minimum) known, which starts the song.
async fn hub(rules: Rules, titles: &[&str], answers: Vec<Answer>) -> Result<Hub> {
    let mut files = HashMap::new();
    for t in titles {
        files.insert(format!("low|{}", low_path(t)), bytes_of("low", &low_path(t), LOW_SIZE));
        let ogg = low_ogg(t).path;
        files.insert(format!("lowogg|{ogg}"), bytes_of("lowogg", &ogg, LOW_SIZE));
    }
    for a in &answers {
        for (path, _) in &a.files {
            files.insert(format!("{}|{path}", a.user), bytes_of(&a.user, path, size_of(&a.user)));
        }
    }
    let low = Peer {
        per_tick: 16_384,
        overwrite: false,
        reported: None,
        settles: Duration::ZERO,
    };
    let fake = Fake::start_peers(files, low, answers)
        .await
        .map_err(|e| Error::internal(e.to_string()))?;
    let db = lull_core::db::Db::memory()?;
    db.migrate(&crate::MIGRATIONS)?;
    let bus = Bus::new();
    let cache = Cache::new(db.clone(), Services::new(bus.clone()), fake.root.clone(), u64::MAX / 2);
    let peers = Peers::load(db.clone()).await?;
    // The formats a hub prefers by default (`LULL_PREFERRED_FORMATS`).
    let preferred = ["flac", "mp3", "opus", "ogg", "m4a"].map(String::from).to_vec();
    let sources = Sources::new(peers.clone(), preferred);
    let searches = Searches::new(fake.slskd.clone(), bus.clone(), db, sources.clone());
    let fetcher = Fetcher::new(
        fake.slskd.clone(),
        Arc::new(RwLock::new(fake.layout.clone())),
        cache,
        peers,
        sources,
        searches,
        bus,
        Current::from(rules),
        Arc::new(NoYoutube),
    );
    Ok(Hub { fake, fetcher })
}

/// A 320 cap: the bands are high lossy and lower lossy, so the race is for a 320.
fn capped() -> Rules {
    let mut rules = Rules::default();
    rules.stream.max_kbps = 320;
    rules
}

fn low_copy(title: &str) -> Source {
    Source {
        user: "low".into(),
        path: low_path(title),
        size: LOW_SIZE as u64,
        ext: "mp3".into(),
        facts: lull_core::model::FileFacts {
            format: "MP3 128".into(),
            codec: "MP3".into(),
            lossless: false,
            khz: Some(44.1),
            bits: None,
            kbps: Some(128),
            size_bytes: Some(LOW_SIZE as u64),
        },
        upload_speed: 500_000,
        free_slot: true,
        queue: 0,
        length: Some(200),
    }
}

/// Another lesser copy ("lowogg", OGG 128), for a preset that streams OGG.
fn low_ogg(title: &str) -> Source {
    let mut s = low_copy(title);
    s.user = "lowogg".into();
    s.path = format!("Music\\Rips\\Album\\01 - {title}.ogg");
    s.ext = "ogg".into();
    s.facts.format = "OGG 128".into();
    s.facts.codec = "OGG".into();
    s
}

/// The song plays its lesser copy; the upgrade searches and races.
async fn play_low(h: &Hub, title: &str) -> Result<Arc<Fetch>> {
    let t = track(title);
    h.fetcher
        .sources
        .add(HashMap::from([(t.id.clone(), vec![low_copy(title)])]));
    let f = h.fetcher.play(t, None).await?;
    let deadline = Instant::now() + SECS(10);
    while f.now().source.as_ref().is_none_or(|s| s.user != "low") {
        if Instant::now() > deadline {
            return Err(Error::internal("the song never started on its lesser copy"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(f)
}

/// The first `upgrade` the player is offered for `f`, and when; None by `within`.
async fn first_offer(h: &Hub, f: &Fetch, within: Duration) -> Option<(Instant, Value)> {
    let deadline = Instant::now() + within;
    loop {
        let up = h.fetcher.event(f)["upgrade"].clone();
        if !up.is_null() {
            return Some((Instant::now(), up));
        }
        if Instant::now() > deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The first offer that is `ready`, and every peer offered before it.
async fn ready_offer(h: &Hub, f: &Fetch, within: Duration) -> (Option<(Instant, Value)>, Vec<String>) {
    let deadline = Instant::now() + within;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let up = h.fetcher.event(f)["upgrade"].clone();
        if let Some(p) = up["peer"].as_str()
            && !seen.iter().any(|s| s == p)
        {
            seen.push(p.to_owned());
        }
        if up["ready"] == true {
            return (Some((Instant::now(), up)), seen);
        }
        if Instant::now() > deadline {
            return (None, seen);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn peer_of(offer: &Value) -> &str {
    offer["peer"].as_str().unwrap_or("none")
}

async fn ended(f: &Arc<Fetch>, within: Duration) -> Result<super::Progress> {
    let mut rx = f.progress();
    let p = tokio::time::timeout(within, rx.wait_for(|p| p.ended()))
        .await
        .map_err(|_| Error::internal("the fetch never ended"))?
        .map_err(|_| Error::internal("gone"))?
        .clone();
    Ok(p)
}

fn since(a: Instant, b: Instant) -> f64 {
    b.saturating_duration_since(a).as_secs_f64()
}

#[tokio::test]
async fn r5_a_fast_peer_wins_over_silent_bursting_and_slow_ones() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            peer("burst", Behaviour::Burst(BURST, SLOW), 8.0, &s),
            peer("slow", Behaviour::Sends(SLOW), 7.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (at, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R5 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "fast", "R5 first offer: {offer}");
    assert_eq!(
        offer["ready"], true,
        "R5 the winner is ready from its first offer: {offer}"
    );
    let t0 = h
        .began(&["silent", "burst", "slow", "fast"])
        .ok_or_else(|| Error::internal("no request"))?;
    assert!(since(t0, at) <= 14.0, "R5 winner after {:.1} s, over 14", since(t0, at));
    Ok(())
}

#[tokio::test]
async fn r6_a_peer_that_queues_then_sends_keeps_its_lane_and_wins() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent1", Behaviour::Silent, 9.0, &s),
            peer("silent2", Behaviour::Silent, 8.0, &s),
            peer("queued", Behaviour::QueuedThen(SECS(2), FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (at, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R6 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "queued", "R6 first offer: {offer}");
    let t0 = h
        .began(&["silent1", "silent2", "queued"])
        .ok_or_else(|| Error::internal("no request"))?;
    assert!(since(t0, at) <= 10.0, "R6 winner after {:.1} s, over 10", since(t0, at));
    assert_eq!(h.asked("queued"), 1, "R6 asked again: it was dropped for being queued");
    Ok(())
}

#[tokio::test]
async fn r7_a_lane_with_no_byte_in_six_seconds_gives_way() -> Result<()> {
    let s = [mp3("Song")];
    let users = ["s1", "s2", "s3", "s4", "fast"];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("s1", Behaviour::Silent, 9.0, &s),
            peer("s2", Behaviour::Silent, 8.0, &s),
            peer("s3", Behaviour::Silent, 7.0, &s),
            peer("s4", Behaviour::Silent, 6.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (_, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R7 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "fast", "R7 first offer: {offer}");
    let t0 = h.began(&users).ok_or_else(|| Error::internal("no request"))?;
    let first_three: Vec<f64> = ["s1", "s2", "s3"]
        .iter()
        .filter_map(|u| h.fake.requests_of(u).first().map(|r| since(t0, r.at)))
        .collect();
    assert!(
        first_three.len() == 3 && first_three.iter().all(|s| *s <= 1.5),
        "R7 three lanes at once, asked at {first_three:?} s"
    );
    let s4 = h
        .fake
        .requests_of("s4")
        .first()
        .map(|r| since(t0, r.at))
        .unwrap_or(-1.0);
    assert!(
        (5.0..=8.5).contains(&s4),
        "R7 the 4th peer asked at {s4:.1} s, not after 6 s with no byte"
    );
    let most = h.fake.most_at_once(&users);
    assert!(most <= 3, "R7 {most} lanes at once");
    Ok(())
}

#[tokio::test]
async fn r8_a_round_asks_at_most_six_peers_then_steps_down_a_band() -> Result<()> {
    let flacs: Vec<String> = (1..=8).map(|n| format!("flac{n}")).collect();
    let fl = [flac("Song")];
    let mut answers: Vec<Answer> = flacs
        .iter()
        .enumerate()
        .map(|(i, u)| peer(u, Behaviour::Silent, 9.0 - i as f64 * 0.5, &fl))
        .collect();
    answers.push(peer("fast320", Behaviour::Sends(FAST), 9.5, &[mp3("Song")]));
    let h = hub(Rules::default(), &["Song"], answers).await?;
    let f = play_low(&h, "Song").await?;
    let (at, offer) = first_offer(&h, &f, SECS(35))
        .await
        .ok_or_else(|| Error::internal("R8 no offer in 35 s"))?;
    assert_eq!(peer_of(&offer), "fast320", "R8 first offer: {offer}");
    let names: Vec<&str> = flacs.iter().map(String::as_str).collect();
    let asked: Vec<&str> = names.iter().copied().filter(|u| h.asked(u) > 0).collect();
    assert_eq!(asked.len(), 6, "R8 lossless peers asked: {asked:?}");
    assert!(
        h.fake.most_at_once(&names) <= 3,
        "R8 more than 3 lossless lanes at once"
    );
    let last_flac = names
        .iter()
        .filter_map(|u| h.fake.requests_of(u).first().map(|r| r.at))
        .max()
        .ok_or_else(|| Error::internal("no lossless request"))?;
    let mp3_at = h
        .fake
        .requests_of("fast320")
        .first()
        .map(|r| r.at)
        .ok_or_else(|| Error::internal("no 320 request"))?;
    assert!(
        since(last_flac, mp3_at) >= 5.0,
        "R8 the 320 asked {:.1} s after the last lossless lane, before its round failed",
        since(last_flac, mp3_at)
    );
    let t0 = h.began(&names).ok_or_else(|| Error::internal("no request"))?;
    assert!(since(t0, at) <= 24.0, "R8 winner after {:.1} s, over 24", since(t0, at));
    Ok(())
}

#[tokio::test]
async fn r9_one_request_per_peer_and_the_winner_is_never_asked_again() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            peer("slow", Behaviour::Sends(SLOW), 8.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (_, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R9 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "fast", "R9 first offer: {offer}");
    let up = h
        .fetcher
        .upgrade(&(track("Song").id, h.profile()))
        .ok_or_else(|| Error::internal("R9 no upgrade fetch"))?;
    let p = ended(&up, SECS(30)).await?;
    assert_eq!(p.status, Status::Done, "R9 the winner's copy: {:?}", p.error);
    for u in ["silent", "slow", "fast"] {
        assert_eq!(h.asked(u), 1, "R9 {u} asked {} times", h.asked(u));
    }
    assert_eq!(h.fake.starts_of("fast"), [0], "R9 the winner's leg is the race's own");
    Ok(())
}

#[tokio::test]
async fn r10_losers_are_cancelled_at_once_and_their_files_forgotten() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("burst", Behaviour::Burst(BURST, SLOW), 9.0, &s),
            peer("slow", Behaviour::Sends(SLOW), 8.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (at, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R10 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "fast", "R10 first offer: {offer}");
    tokio::time::sleep(SECS(2)).await;
    let t = track("Song");
    for loser in ["burst", "slow"] {
        let r = h.fake.requests_of(loser);
        let ended = r.first().and_then(|r| r.ended);
        assert!(
            r.first().is_some_and(|r| r.cancelled) && ended.is_some_and(|e| since(at, e) <= 1.5),
            "R10 {loser} not cancelled within 1.5 s of the win: {r:?}"
        );
        let copy = the(loser, &mp3("Song"));
        assert!(
            h.fetcher.cache.partial(&t.id, &copy).await?.is_none(),
            "R10 {loser}'s part is cached"
        );
        assert!(
            h.fetcher.cache.finished(&t.id, &copy).await?.is_none(),
            "R10 {loser}'s copy is cached"
        );
        let file = h.fetcher.cache.copy_file(&copy).await?;
        assert!(
            !file.exists(),
            "R10 {loser}'s file is still on disk: {}",
            file.display()
        );
        assert!(
            h.fetcher.copy(&t.id, &h.profile(), &copy).is_err(),
            "R10 {loser}'s copy can still be served"
        );
    }
    Ok(())
}

#[tokio::test]
async fn r11_the_gate_counts_bytes_not_the_speed_slskd_reports() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("liar", Behaviour::Sends(SLOW), 9.0, &s).reports(10_000_000.0),
            peer("honest", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (offer, seen) = ready_offer(&h, &f, SECS(25)).await;
    let (_, offer) = offer.ok_or_else(|| Error::internal("R11 nothing ready in 25 s"))?;
    assert_eq!(seen, ["honest"], "R11 peers offered: {seen:?}");
    assert_eq!(peer_of(&offer), "honest", "R11 ready offer: {offer}");
    Ok(())
}

#[tokio::test]
async fn r12_one_switch_per_play() -> Result<()> {
    let fl = [flac("Song")];
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            peer("flac1", Behaviour::Silent, 9.0, &fl),
            peer("flac2", Behaviour::QueuedThen(SECS(20), FAST), 8.0, &fl),
            peer("fast320", Behaviour::Sends(FAST), 1.0, &[mp3("Song")]),
        ],
    )
    .await?;
    let t = track("Song");
    let f = play_low(&h, "Song").await?;
    let (_, offer) = first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R12 no offer in 25 s"))?;
    assert_eq!(peer_of(&offer), "fast320", "R12 first offer: {offer}");
    h.fetcher
        .adopt(&t.id, &h.profile(), &the("fast320", &mp3("Song")), false)?;
    // The player's event stream reconnects: the song is opened again.
    let again = h.fetcher.play(t.clone(), None).await?;
    let asked = h.fake.requests().len();
    tokio::time::sleep(SECS(4)).await;
    let later: Vec<String> = h.fake.requests().into_iter().skip(asked).map(|r| r.user).collect();
    assert!(later.is_empty(), "R12 asked after the switch: {later:?}");
    assert!(
        h.fetcher.event(&again)["upgrade"].is_null(),
        "R12 another better copy offered after the switch"
    );
    Ok(())
}

#[tokio::test]
async fn r13_with_no_band_sending_one_queue_is_kept_and_left_at_the_end() -> Result<()> {
    let fl = [flac("Song")];
    let m = [mp3("Song")];
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            peer("q7", Behaviour::Queues, 9.0, &fl).busy(7),
            peer("q2", Behaviour::Queues, 5.0, &fl).busy(2),
            peer("h1", Behaviour::Queues, 9.0, &m).busy(1),
            peer("h3", Behaviour::Queues, 8.0, &m).busy(3),
        ],
    )
    .await?;
    let t = track("Song");
    let f = play_low(&h, "Song").await?;
    let deadline = Instant::now() + SECS(10);
    let t0 = loop {
        if let Some(t0) = h.began(&["q7", "q2", "h1", "h3"]) {
            break t0;
        }
        if Instant::now() > deadline {
            return Err(Error::internal("R13 no peer asked in 10 s"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::time::sleep((t0 + SECS(16)).saturating_duration_since(Instant::now())).await;
    for u in ["q7", "q2", "h1", "h3"] {
        assert_eq!(h.asked(u), 1, "R13 {u} asked {} times", h.asked(u));
    }
    assert_eq!(h.fake.running(), ["q2"], "R13 queues kept");
    let note = f.now().note.unwrap_or_default();
    assert!(note.starts_with("Stays on this copy: "), "R13 note: {note:?}");
    h.fetcher.stop_streaming(&t.id, &h.profile());
    tokio::time::sleep(SECS(2)).await;
    assert!(
        h.fake.running().is_empty(),
        "R13 still queued after the song: {:?}",
        h.fake.running()
    );
    Ok(())
}

#[tokio::test]
async fn r14_a_kept_queue_that_sends_runs_on_after_the_song() -> Result<()> {
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            peer("q2", Behaviour::QueuedThen(SECS(15), SLOW), 5.0, &[flac("Song")]).busy(2),
            peer("h1", Behaviour::Queues, 9.0, &[mp3("Song")]).busy(1),
        ],
    )
    .await?;
    let t = track("Song");
    play_low(&h, "Song").await?;
    let deadline = Instant::now() + SECS(10);
    let t0 = loop {
        if let Some(t0) = h.began(&["q2", "h1"]) {
            break t0;
        }
        if Instant::now() > deadline {
            return Err(Error::internal("R14 no peer asked in 10 s"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::time::sleep((t0 + SECS(18)).saturating_duration_since(Instant::now())).await;
    assert_eq!(h.fake.running(), ["q2"], "R14 the kept queue, sending now");
    h.fetcher.stop_streaming(&t.id, &h.profile());
    tokio::time::sleep(SECS(3)).await;
    assert_eq!(h.fake.running(), ["q2"], "R14 the copy sending stops with the song");
    assert_eq!(h.asked("q2"), 1, "R14 q2 asked {} times", h.asked("q2"));
    Ok(())
}

#[tokio::test]
async fn r15_two_songs_racing_share_four_lanes() -> Result<()> {
    let (a, b) = ([mp3("Alpha")], [mp3("Bravo")]);
    let users = ["a1", "a2", "a3", "b1", "b2", "b3"];
    let h = hub(
        capped(),
        &["Alpha", "Bravo"],
        vec![
            peer("a1", Behaviour::Silent, 9.0, &a),
            peer("a2", Behaviour::Silent, 8.0, &a),
            peer("a3", Behaviour::Silent, 7.0, &a),
            peer("b1", Behaviour::Silent, 9.0, &b),
            peer("b2", Behaviour::Silent, 8.0, &b),
            peer("b3", Behaviour::Silent, 7.0, &b),
        ],
    )
    .await?;
    play_low(&h, "Alpha").await?;
    play_low(&h, "Bravo").await?;
    let deadline = Instant::now() + SECS(10);
    let t0 = loop {
        if let Some(t0) = h.began(&users) {
            break t0;
        }
        if Instant::now() > deadline {
            return Err(Error::internal("R15 no peer asked in 10 s"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    tokio::time::sleep((t0 + SECS(4)).saturating_duration_since(Instant::now())).await;
    let running = h.fake.running().len();
    assert_eq!(running, 4, "R15 lanes running across both songs: {running}");
    tokio::time::sleep(SECS(10)).await;
    let most = h.fake.most_at_once(&users);
    assert!(most <= 4, "R15 {most} lanes at once");
    Ok(())
}

#[tokio::test]
async fn r16_playing_songs_get_lanes_before_a_preload() -> Result<()> {
    let (a, b) = ([mp3("Alpha")], [mp3("Bravo")]);
    let playing = ["a1", "a2", "a3", "b1", "b2", "b3"];
    let h = hub(
        capped(),
        &["Alpha", "Bravo"],
        vec![
            peer("a1", Behaviour::Silent, 9.0, &a),
            peer("a2", Behaviour::Silent, 8.0, &a),
            peer("a3", Behaviour::Silent, 7.0, &a),
            peer("b1", Behaviour::Silent, 9.0, &b),
            peer("b2", Behaviour::Silent, 8.0, &b),
            peer("b3", Behaviour::Silent, 7.0, &b),
            peer("c1", Behaviour::Sends(FAST), 9.0, &[mp3("Charlie")]),
        ],
    )
    .await?;
    play_low(&h, "Alpha").await?;
    play_low(&h, "Bravo").await?;
    // Both songs' races hold every lane before the preload asks for one.
    let deadline = Instant::now() + SECS(10);
    while h.fake.running().len() < 4 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let pre = h.fetcher.prefetch(track("Charlie"), &h.profile());
    ended(&pre, SECS(40)).await?;
    let c1 = h
        .fake
        .requests_of("c1")
        .first()
        .map(|r| r.at)
        .ok_or_else(|| Error::internal("R16 c1 never asked"))?;
    // Lanes freed at one moment go to the playing songs first; 1 s of slack for scheduling.
    let late: Vec<&str> = playing
        .iter()
        .copied()
        .filter(|u| h.fake.requests_of(u).first().is_some_and(|r| since(c1, r.at) > 1.0))
        .collect();
    assert!(
        late.is_empty(),
        "R16 playing songs' peers asked after the preload's: {late:?}"
    );
    Ok(())
}

#[tokio::test]
async fn r17_a_preload_asks_one_peer_at_a_time_for_the_top_band() -> Result<()> {
    let fl = [flac("Delta")];
    let users = ["fsilent", "ffast", "h320"];
    let h = hub(
        Rules::default(),
        &["Delta"],
        vec![
            peer("fsilent", Behaviour::Silent, 9.0, &fl),
            peer("ffast", Behaviour::Sends(FAST), 1.0, &fl),
            peer("h320", Behaviour::Sends(FAST), 9.5, &[mp3("Delta")]),
        ],
    )
    .await?;
    let pre = h.fetcher.prefetch(track("Delta"), &h.profile());
    let p = ended(&pre, SECS(40)).await?;
    let first = h
        .fake
        .requests()
        .first()
        .map(|r| (r.user.clone(), r.path.clone()))
        .unwrap_or_default();
    assert_eq!(first, ("fsilent".to_owned(), flac("Delta")), "R17 first asked");
    assert_eq!(
        h.asked("h320"),
        0,
        "R17 a lower band asked while the top band had peers"
    );
    assert_eq!(h.fake.most_at_once(&users), 1, "R17 more than one lane for a preload");
    let gap = match (
        h.fake.requests_of("fsilent").first(),
        h.fake.requests_of("ffast").first(),
    ) {
        (Some(a), Some(b)) => since(a.at, b.at),
        _ => -1.0,
    };
    assert!(
        (5.0..=8.5).contains(&gap),
        "R17 next peer asked {gap:.1} s after the silent one"
    );
    assert_eq!(p.source.map(|s| s.user).as_deref(), Some("ffast"), "R17 preloaded copy");
    Ok(())
}

#[tokio::test]
async fn r18_a_peer_cancelled_three_times_today_is_not_raced() -> Result<()> {
    let titles = ["Alpha", "Bravo", "Charlie", "Delta"];
    let files: Vec<String> = titles.iter().map(|t| mp3(t)).collect();
    let h = hub(
        capped(),
        &titles,
        vec![
            peer("loser", Behaviour::Sends(SLOW), 9.0, &files),
            peer("winner", Behaviour::Sends(FASTEST), 1.0, &files),
        ],
    )
    .await?;
    // Each song's search finds only its own file (the stand-in matches as Soulseek does), so
    // every song starts on its lesser copy and races the same two peers.
    for title in titles {
        let f = play_low(&h, title).await?;
        let (_, offer) = first_offer(&h, &f, SECS(25))
            .await
            .ok_or_else(|| Error::internal(format!("R18 no offer for {title}")))?;
        assert_eq!(peer_of(&offer), "winner", "R18 {title}: {offer}");
        // The winner's copy all arrives before the song moves on: only losers are cancelled.
        if let Some(up) = h.fetcher.upgrade(&(track(title).id, h.profile())) {
            ended(&up, SECS(20)).await?;
        }
        h.fetcher.stop_streaming(&track(title).id, &h.profile());
        tokio::time::sleep(SECS(1)).await;
    }
    let asked = h.fake.requests_of("loser");
    assert_eq!(asked.len(), 3, "R18 the loser was raced {} times", asked.len());
    assert!(
        asked.iter().all(|r| r.cancelled && r.sent),
        "R18 each a sending upload cancelled: {asked:?}"
    );
    Ok(())
}

#[tokio::test]
async fn r19_a_pick_that_queues_is_waited_for() -> Result<()> {
    let h = hub(
        capped(),
        &["Song"],
        vec![peer(
            "patient",
            Behaviour::QueuedThen(SECS(8), FAST),
            1.0,
            &[mp3("Song")],
        )],
    )
    .await?;
    let f = h
        .fetcher
        .play(track("Song"), Some(the("patient", &mp3("Song"))))
        .await?;
    let p = ended(&f, SECS(40)).await?;
    assert_eq!(p.status, Status::Done, "R19 {:?}", p.error);
    assert_eq!(p.source.map(|s| s.user).as_deref(), Some("patient"), "R19 played");
    assert_eq!(h.asked("patient"), 1, "R19 the pick was asked again");
    assert!(p.failed_peers.is_empty(), "R19 the pick fell back");
    Ok(())
}

#[tokio::test]
async fn r20_a_songs_start_moves_past_a_silent_peer_in_six_seconds() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let opened = Instant::now();
    let f = h.fetcher.play(track("Song"), None).await?;
    let deadline = opened + SECS(15);
    while !f.now().ranges.has(0) {
        if Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let at = since(opened, Instant::now());
    let first = h.fake.requests().first().map(|r| r.user.clone()).unwrap_or_default();
    assert_eq!(first, "silent", "R20 first asked");
    assert!(at <= 10.0, "R20 first byte after {at:.1} s, over 10");
    Ok(())
}

#[tokio::test]
async fn s1_the_race_leaves_the_playing_copy_alone() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            peer("slow", Behaviour::Sends(SLOW), 8.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let t = track("Song");
    let f = play_low(&h, "Song").await?;
    let (job, attempt) = (f.job_id.clone(), f.now().attempt);
    let until = Instant::now() + SECS(12);
    while Instant::now() < until {
        let p = f.now();
        let user = p.source.map(|s| s.user).unwrap_or_default();
        assert!(
            user == "low" && p.attempt == attempt,
            "S1 the playing fetch moved: {user}, attempt {}",
            p.attempt
        );
        assert_eq!(h.fetcher.event(&f)["jobId"], job.as_str(), "S1 the playing job changed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let again = h.fetcher.play(t, None).await?;
    assert!(
        Arc::ptr_eq(&again, &f),
        "S1 reopening the song moved it to another fetch"
    );
    Ok(())
}

#[tokio::test]
async fn s2_after_the_switch_a_seek_anywhere_reads_the_winners_bytes() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let t = track("Song");
    let f = play_low(&h, "Song").await?;
    let (offer, _) = ready_offer(&h, &f, SECS(30)).await;
    let (_, offer) = offer.ok_or_else(|| Error::internal("S2 nothing ready in 30 s"))?;
    assert_eq!(peer_of(&offer), "fast", "S2 ready offer: {offer}");
    let live = h
        .fetcher
        .adopt(&t.id, &h.profile(), &the("fast", &mp3("Song")), false)?;
    let byte = (SIZE * 3 / 4) as u64;
    let _asking = live.ask_for(byte);
    let deadline = Instant::now() + SECS(20);
    while !live.now().ranges.has(byte + 63) {
        if Instant::now() > deadline {
            return Err(Error::internal("S2 the seek's bytes never arrived"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let path = live.now().path.ok_or_else(|| Error::internal("S2 no file"))?;
    let file = std::fs::read(&path).map_err(|e| Error::internal(e.to_string()))?;
    let want = bytes_of("fast", &mp3("Song"), SIZE);
    let at = byte as usize;
    assert!(
        file.get(at..at + 64) == want.get(at..at + 64),
        "S2 the bytes at the seek aren't the winner's"
    );
    assert_eq!(h.fake.starts_of("fast").first(), Some(&0), "S2 the winner's first leg");
    Ok(())
}

// Review round 1 (ADR-0018, R22-R28): what the first build got wrong that the tests above missed.

fn profile(formats: &[&str], max_kbps: u32) -> StreamProfile {
    let mut p = StreamProfile::default();
    p.minimum.formats = formats.iter().map(|f| (*f).to_owned()).collect();
    p.max_kbps = max_kbps;
    p
}

/// Waits until `f` plays a copy of `user`'s.
async fn on(f: &Fetch, user: &str, within: Duration) -> Result<()> {
    let deadline = Instant::now() + within;
    while f.now().source.as_ref().is_none_or(|s| s.user != user) {
        if Instant::now() > deadline {
            return Err(Error::internal(format!("never started on {user}'s copy")));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

/// The ready offer for the song a device of `profile` plays, if any by `within`.
async fn ready_for(h: &Hub, f: &Fetch, within: Duration) -> Option<Value> {
    ready_offer(h, f, within).await.0.map(|(_, offer)| offer)
}

#[tokio::test]
async fn r22_two_presets_racing_one_song_never_share_a_peer_and_file() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            peer("fast1", Behaviour::Sends(FAST), 9.0, &s),
            peer("fast2", Behaviour::Sends(FAST), 8.0, &s),
            peer("silent", Behaviour::Silent, 7.0, &s),
        ],
    )
    .await?;
    let t = track("Song");
    // Two devices, two presets, each on a lesser copy of its own, racing for the same 320s.
    let (ogg, mp3_only) = (profile(&["ogg", "mp3"], 0), profile(&["mp3"], 320));
    h.fetcher
        .sources
        .add(HashMap::from([(t.id.clone(), vec![low_ogg("Song")])]));
    let a = h.fetcher.stream(t.clone(), None, &ogg).await?;
    on(&a, "lowogg", SECS(10)).await?;
    h.fetcher
        .sources
        .add(HashMap::from([(t.id.clone(), vec![low_copy("Song")])]));
    let b = h.fetcher.stream(t.clone(), None, &mp3_only).await?;
    on(&b, "low", SECS(10)).await?;
    let (offer_a, offer_b) = tokio::join!(ready_for(&h, &a, SECS(30)), ready_for(&h, &b, SECS(30)));
    assert_eq!(
        h.fake.twice_at_once(),
        Vec::<String>::new(),
        "R22 one copy asked twice at once"
    );
    for (device, offer, p) in [("ogg", offer_a, &ogg), ("mp3", offer_b, &mp3_only)] {
        let offer = offer.ok_or_else(|| Error::internal(format!("R22 nothing ready for the {device} preset")))?;
        let user = peer_of(&offer).to_owned();
        let up = h
            .fetcher
            .upgrade(&(t.id.clone(), p.clone()))
            .ok_or_else(|| Error::internal(format!("R22 the {device} preset's copy is gone")))?;
        let done = ended(&up, SECS(30)).await?;
        assert_eq!(done.status, Status::Done, "R22 {device}: {:?}", done.error);
        let path = done.path.ok_or_else(|| Error::internal("R22 no file"))?;
        let file = std::fs::read(&path).unwrap_or_default();
        assert!(
            file == bytes_of(&user, &mp3("Song"), SIZE),
            "R22 the {device} preset's copy from {user} isn't whole ({} bytes)",
            file.len()
        );
    }
    Ok(())
}

#[tokio::test]
async fn r23_a_lane_never_deletes_a_copy_already_finished() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("finished", Behaviour::Sends(SLOW), 9.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let t = track("Song");
    let f = play_low(&h, "Song").await?;
    // Meanwhile another preset's fetch finished a copy of `finished`'s file into the cache.
    let copy = the("finished", &mp3("Song"));
    let path = h.fetcher.cache.copy_file(&copy).await?;
    let bytes = bytes_of("finished", &mp3("Song"), SIZE);
    std::fs::write(&path, &bytes).map_err(|e| Error::internal(e.to_string()))?;
    h.fetcher
        .cache
        .record(crate::cache::Entry {
            track: t.id.clone(),
            source: copy.clone(),
            path: path.clone(),
            complete: true,
            used_at: lull_core::now(),
            ranges: None,
        })
        .await?;
    first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R23 no offer in 25 s"))?;
    tokio::time::sleep(SECS(2)).await;
    assert!(
        h.fetcher.cache.finished(&t.id, &copy).await?.is_some(),
        "R23 the finished copy left the cache"
    );
    let now = std::fs::read(&path).unwrap_or_default();
    assert!(
        now == bytes,
        "R23 the finished copy's file was changed ({} bytes)",
        now.len()
    );
    Ok(())
}

/// A song that played `half`'s copy part of the way, then was skipped: a partial in the cache.
async fn skipped_part_way(h: &Hub, title: &str, user: &str) -> Result<()> {
    let f = h.fetcher.play(track(title), None).await?;
    on(&f, user, SECS(15)).await?;
    let deadline = Instant::now() + SECS(15);
    while f.now().received() < 300_000 {
        if Instant::now() > deadline {
            return Err(Error::internal(format!("{title}: the first part never came")));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    h.fetcher.stop_streaming(&track(title).id, &h.profile());
    Ok(())
}

#[tokio::test]
async fn r24_a_start_resuming_a_partial_moves_past_a_silent_peer_in_six_seconds() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            // Sends the first 300 KB to anyone, then nothing more: a resumed request gets no byte.
            peer("half", Behaviour::Until(300_000, FAST), 9.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    skipped_part_way(&h, "Song", "half").await?;
    tokio::time::sleep(SECS(1)).await;
    let opened = Instant::now();
    let f = h.fetcher.play(track("Song"), None).await?;
    let deadline = opened + SECS(15);
    while !f.now().source.is_some_and(|s| s.user == "fast") || !f.now().ranges.has(0) {
        if Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let at = since(opened, Instant::now());
    let resumed = h.fake.requests_of("half").iter().any(|r| r.offset > 0);
    assert!(resumed, "R24 the partial wasn't resumed");
    // No search (the copies are known): 6 s for the silent resume, then the next peer's first byte.
    assert!(at <= 8.5, "R24 the next peer's bytes after {at:.1} s, over 8.5");
    Ok(())
}

#[tokio::test]
async fn r25_starts_resuming_partials_count_in_the_lane_budget() -> Result<()> {
    let titles = ["Alpha", "Bravo", "Charlie", "Delta", "Echo"];
    let users = ["ha", "hb", "hc", "hd", "he"];
    let answers = titles
        .iter()
        .zip(users)
        .map(|(t, u)| peer(u, Behaviour::Until(300_000, FAST), 9.0, &[mp3(t)]))
        .collect();
    let h = hub(capped(), &titles, answers).await?;
    for (t, u) in titles.iter().zip(users) {
        skipped_part_way(&h, t, u).await?;
    }
    tokio::time::sleep(SECS(1)).await;
    let before = h.fake.requests().len();
    for t in titles {
        h.fetcher.play(track(t), None).await?;
    }
    tokio::time::sleep(SECS(5)).await;
    let again: Vec<_> = h.fake.requests().into_iter().skip(before).collect();
    assert!(
        again.iter().all(|r| r.offset > 0),
        "R25 a partial wasn't resumed: {again:?}"
    );
    let most = again
        .iter()
        .map(|r| {
            again
                .iter()
                .filter(|o| o.at <= r.at && o.ended.is_none_or(|e| e > r.at))
                .count()
        })
        .max()
        .unwrap_or(0);
    assert!(most <= 4, "R25 {most} resumed requests waiting at once, over 4 lanes");
    Ok(())
}

/// The INFO lines logged on this test's thread: message and fields.
#[derive(Clone, Default)]
struct Lines(Arc<std::sync::Mutex<Vec<HashMap<String, String>>>>);

struct Fields<'a>(&'a mut HashMap<String, String>);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}

impl tracing::Subscriber for Lines {
    fn enabled(&self, m: &tracing::Metadata<'_>) -> bool {
        *m.level() <= tracing::Level::INFO
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, e: &tracing::Event<'_>) {
        if *e.metadata().level() != tracing::Level::INFO {
            return;
        }
        let mut fields = HashMap::new();
        e.record(&mut Fields(&mut fields));
        if let Ok(mut lines) = self.0.lock() {
            lines.push(fields);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

impl Lines {
    /// The INFO lines about the better copy (its race, or fetching it).
    fn about_the_better_copy(&self) -> Vec<HashMap<String, String>> {
        let lines = self.0.lock().map(|l| l.clone()).unwrap_or_default();
        lines
            .into_iter()
            .filter(|l| {
                let m = l.get("message").map(String::as_str).unwrap_or_default();
                m.contains("better copy") || m.contains("better-copy")
            })
            .collect()
    }
}

#[tokio::test]
async fn r26_a_won_race_leaves_one_log_line_with_its_time_to_winner() -> Result<()> {
    let lines = Lines::default();
    let _logging = tracing::subscriber::set_default(lines.clone());
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("slow", Behaviour::Sends(SLOW), 9.0, &s),
            peer("silent", Behaviour::Silent, 8.0, &s),
            peer("fast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    // slskd takes 3 s to answer each cancel: the time to the winner doesn't include them.
    h.fake.slow_cancels(SECS(3));
    first_offer(&h, &f, SECS(25))
        .await
        .ok_or_else(|| Error::internal("R26 no offer in 25 s"))?;
    tokio::time::sleep(SECS(4)).await;
    let said = lines.about_the_better_copy();
    assert_eq!(said.len(), 1, "R26 lines about the better copy: {said:?}");
    let line = &said[0];
    assert_eq!(
        line.get("message").map(String::as_str),
        Some("better-copy race"),
        "R26 {line:?}"
    );
    assert_eq!(line.get("winner").map(String::as_str), Some("fast"), "R26 {line:?}");
    let secs: f64 = line.get("secs").and_then(|s| s.parse().ok()).unwrap_or(f64::MAX);
    assert!(
        secs <= 4.5,
        "R26 time to winner {secs:.1} s: it counts more than the race"
    );
    Ok(())
}

#[tokio::test]
async fn r26_an_interrupted_race_leaves_one_log_line() -> Result<()> {
    let lines = Lines::default();
    let _logging = tracing::subscriber::set_default(lines.clone());
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("s1", Behaviour::Silent, 9.0, &s),
            peer("s2", Behaviour::Silent, 8.0, &s),
        ],
    )
    .await?;
    play_low(&h, "Song").await?;
    let deadline = Instant::now() + SECS(10);
    while h.began(&["s1", "s2"]).is_none() {
        if Instant::now() > deadline {
            return Err(Error::internal("R26 no peer asked in 10 s"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(SECS(2)).await;
    // The listener skips while the lanes race.
    h.fetcher.stop_streaming(&track("Song").id, &h.profile());
    tokio::time::sleep(SECS(3)).await;
    let said = lines.about_the_better_copy();
    assert_eq!(said.len(), 1, "R26 lines about the better copy: {said:?}");
    assert_eq!(
        said[0].get("message").map(String::as_str),
        Some("better-copy race"),
        "R26 {said:?}"
    );
    assert_eq!(said[0].get("winner").map(String::as_str), Some("none"), "R26 {said:?}");
    Ok(())
}

#[tokio::test]
async fn r27_a_peer_dropping_the_request_itself_is_not_counted_as_a_cancel() -> Result<()> {
    let titles = ["Alpha", "Bravo", "Charlie", "Delta"];
    let files: Vec<String> = titles.iter().map(|t| mp3(t)).collect();
    let h = hub(
        capped(),
        &titles,
        vec![
            // Sends 200 KB, then drops the request itself (slskd: Completed, Errored).
            peer("dropper", Behaviour::Drops(200_000, FAST), 9.0, &files),
            peer("winner", Behaviour::Sends(FASTEST), 1.0, &files),
        ],
    )
    .await?;
    for title in titles {
        let f = play_low(&h, title).await?;
        let (_, offer) = first_offer(&h, &f, SECS(25))
            .await
            .ok_or_else(|| Error::internal(format!("R27 no offer for {title}")))?;
        assert_eq!(peer_of(&offer), "winner", "R27 {title}: {offer}");
        if let Some(up) = h.fetcher.upgrade(&(track(title).id, h.profile())) {
            ended(&up, SECS(20)).await?;
        }
        h.fetcher.stop_streaming(&track(title).id, &h.profile());
        tokio::time::sleep(SECS(1)).await;
    }
    let asked = h.fake.requests_of("dropper");
    assert!(
        asked.iter().all(|r| !r.cancelled),
        "R27 lull cancelled the dropper: {asked:?}"
    );
    assert_eq!(asked.len(), 4, "R27 the dropper was raced {} times, not 4", asked.len());
    Ok(())
}

#[tokio::test]
async fn r28_a_kept_copy_that_arrives_whole_is_offered() -> Result<()> {
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            // Queued 7 s, then slow for 17 s (long enough to be judged slow), then fast to the end.
            peer(
                "q",
                Behaviour::Slowly {
                    queued: SECS(7),
                    slow_for: SECS(17),
                    slow: SLOW,
                    then: FASTEST,
                },
                5.0,
                &[flac("Song")],
            )
            .busy(2),
            peer("h1", Behaviour::Queues, 9.0, &[mp3("Song")]).busy(1),
        ],
    )
    .await?;
    let f = play_low(&h, "Song").await?;
    let (offer, seen) = ready_offer(&h, &f, SECS(50)).await;
    let (_, offer) = offer.ok_or_else(|| Error::internal(format!("R28 nothing ready in 50 s; offered {seen:?}")))?;
    assert_eq!(peer_of(&offer), "q", "R28 ready offer: {offer}");
    Ok(())
}

// Final review (ADR-0018, R29-R30).

#[tokio::test]
async fn r29_a_songs_start_never_takes_a_copy_a_race_holds() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        Rules::default(),
        &["Song"],
        vec![
            // A's race takes x, y and w; w is all here first and wins, x and y are cancelled.
            peer("x", Behaviour::Sends(FAST), 9.0, &s),
            peer("y", Behaviour::Sends(FAST), 8.0, &s),
            peer("w", Behaviour::Sends(FASTEST), 1.0, &s),
            peer("z", Behaviour::Sends(FAST), 0.5, &s),
        ],
    )
    .await?;
    let t = track("Song");
    let (ogg, mp3_only) = (profile(&["ogg", "mp3"], 0), profile(&["mp3"], 320));
    // Device A plays an OGG 128 and races for a 320.
    h.fetcher
        .sources
        .add(HashMap::from([(t.id.clone(), vec![low_ogg("Song")])]));
    let a = h.fetcher.stream(t.clone(), None, &ogg).await?;
    on(&a, "lowogg", SECS(10)).await?;
    let deadline = Instant::now() + SECS(10);
    while h.began(&["x", "y", "w"]).is_none() {
        if Instant::now() > deadline {
            return Err(Error::internal("R29 A's race never began"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Device B (MP3 only) starts the song while A's lanes run: it can't join A's OGG.
    let b = h.fetcher.stream(t.clone(), None, &mp3_only).await?;
    let p = match ended(&b, SECS(40)).await {
        Ok(p) => p,
        Err(_) => {
            let p = b.now();
            let user = p.source.as_ref().map(|s| s.user.clone()).unwrap_or_default();
            let file = p.path.as_ref().map(|f| std::fs::metadata(f).map(|m| m.len()).ok());
            return Err(Error::internal(format!(
                "R29 B's song on {user} never ended in 40 s: {:?}, {} of {} bytes in, its file {file:?}",
                p.status,
                p.received(),
                p.size()
            )));
        }
    };
    let user = p.source.as_ref().map(|s| s.user.clone()).unwrap_or_default();
    assert_eq!(p.status, Status::Done, "R29 B's song on {user}: {:?}", p.error);
    let path = p.path.ok_or_else(|| Error::internal("R29 B's copy has no file"))?;
    let file = std::fs::read(&path).unwrap_or_default();
    assert!(
        file == bytes_of(&user, &mp3("Song"), SIZE),
        "R29 B's copy from {user} isn't whole ({} bytes at {})",
        file.len(),
        path.display()
    );
    Ok(())
}

#[tokio::test]
async fn r30_a_seek_before_the_first_byte_keeps_the_six_second_rule() -> Result<()> {
    let s = [mp3("Song")];
    let h = hub(
        capped(),
        &["Song"],
        vec![
            peer("silent", Behaviour::Silent, 9.0, &s),
            // Another rip (another size): no side leg on it, only a fallback to it.
            peer("ripfast", Behaviour::Sends(FAST), 1.0, &s),
        ],
    )
    .await?;
    let f = h.fetcher.play(track("Song"), None).await?;
    on(&f, "silent", SECS(15)).await?;
    let t0 = h
        .began(&["silent"])
        .ok_or_else(|| Error::internal("R30 silent never asked"))?;
    // The listener seeks to three quarters before any byte came.
    let byte = (SIZE * 3 / 4) as u64;
    let _seek = f.ask_for(byte);
    let deadline = t0 + SECS(20);
    while !(f.now().source.is_some_and(|s| s.user == "ripfast") && f.now().received() > 0) {
        if Instant::now() > deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let at = since(t0, Instant::now());
    let silent = h.fake.requests_of("silent");
    let mut from: Vec<u64> = silent.iter().map(|r| r.offset).collect();
    from.dedup();
    assert!(
        at <= 8.5,
        "R30 the next peer's bytes after {at:.1} s, over 8.5 (silent asked {} times, from {from:?})",
        silent.len()
    );
    let landed = Instant::now() + SECS(10);
    while !f.now().ranges.has(byte) && Instant::now() < landed {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(f.now().ranges.has(byte), "R30 the seek's byte never arrived");
    Ok(())
}
