//! What lull learned about peers: those that stalled on us, demoted for a while (a finished
//! transfer clears it), and how fast each one actually sent (ADR-0015), which ranking trusts
//! over the speed a peer reports.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use lull_core::Result;
use lull_core::db::Db;
use rusqlite::params;

/// A stall counts against a peer for this long.
const MEMORY_SECS: i64 = 7 * 24 * 3600;
/// A measurement's weight halves every this long: a peer's connection, load and slots change
/// over days, and a week keeps a weekly listener's peers known (ADR-0015).
const HALF_LIFE_SECS: f64 = 7.0 * 24.0 * 3600.0;
/// Reported speeds are calibrated once this many peers have both a reported and a measured
/// rate: fewer, and one odd peer would set the scale for all (ADR-0015).
const CALIBRATE_AFTER: usize = 5;

/// What lull measured from a peer: bytes a second, how much the average weighs now, what the
/// peer reported then, and when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Speed {
    pub rate: f64,
    pub weight: f64,
    pub reported: f64,
    pub at: i64,
}

impl Speed {
    /// The average with a new measurement at `now`: the old weight halved by its age, the new one 1.
    fn with(self, rate: f64, reported: f64, now: i64) -> Speed {
        let old = self.weight * 0.5f64.powf((now - self.at).max(0) as f64 / HALF_LIFE_SECS);
        Speed {
            rate: (self.rate * old + rate) / (old + 1.0),
            weight: old + 1.0,
            reported: if reported > 0.0 { reported } else { self.reported },
            at: now,
        }
    }
}

#[derive(Clone)]
pub struct Peers {
    db: Db,
    /// user → unix time of their last stall; mirrors `soulseek_peers`.
    stalled: Arc<RwLock<HashMap<String, i64>>>,
    /// user → how fast it sent; mirrors `soulseek_speeds`.
    speeds: Arc<RwLock<HashMap<String, Speed>>>,
}

impl Peers {
    pub async fn load(db: Db) -> Result<Self> {
        let since = lull_core::now() - MEMORY_SECS;
        let (rows, speeds) = db
            .call(move |c| {
                c.execute("DELETE FROM soulseek_peers WHERE stalled_at < ?1", [since])?;
                let mut stmt = c.prepare("SELECT user, stalled_at FROM soulseek_peers")?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                let rows = rows.collect::<Result<HashMap<String, i64>, _>>()?;
                let mut stmt = c.prepare("SELECT user, rate, weight, reported, measured_at FROM soulseek_speeds")?;
                let speeds = stmt.query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        Speed {
                            rate: r.get(1)?,
                            weight: r.get(2)?,
                            reported: r.get(3)?,
                            at: r.get(4)?,
                        },
                    ))
                })?;
                Ok((rows, speeds.collect::<Result<HashMap<String, Speed>, _>>()?))
            })
            .await?;
        Ok(Peers {
            db,
            stalled: Arc::new(RwLock::new(rows)),
            speeds: Arc::new(RwLock::new(speeds)),
        })
    }

    /// How fast `user` sent to this hub, when lull has measured it.
    pub fn measured(&self, user: &str) -> Option<Speed> {
        self.speeds.read().ok()?.get(user).copied()
    }

    /// The speed to rank a peer by, in bytes a second, and whether lull measured it: what it sent,
    /// else what it reports, scaled by how reports have compared to what peers sent here.
    pub fn expected(&self, user: &str, reported: u64) -> (f64, Option<Speed>) {
        match self.measured(user) {
            Some(s) => (s.rate, Some(s)),
            None => (reported as f64 * self.calibration(), None),
        }
    }

    /// The median of measured over reported, over peers with both, once there are enough of them.
    fn calibration(&self) -> f64 {
        let Ok(speeds) = self.speeds.read() else { return 1.0 };
        let mut ratios: Vec<f64> = speeds
            .values()
            .filter(|s| s.reported > 0.0)
            .map(|s| s.rate / s.reported)
            .collect();
        if ratios.len() < CALIBRATE_AFTER {
            return 1.0;
        }
        ratios.sort_by(f64::total_cmp);
        ratios[ratios.len() / 2]
    }

    /// Adds a transfer's rate (bytes a second) to what lull knows of `user`, with the speed it
    /// reported (0: unknown).
    pub async fn sent(&self, user: &str, rate: f64, reported: u64) -> Result<()> {
        let now = lull_core::now();
        let fresh = Speed {
            rate,
            weight: 1.0,
            reported: reported as f64,
            at: now,
        };
        let speed = {
            let Ok(mut m) = self.speeds.write() else { return Ok(()) };
            let speed = m.get(user).map_or(fresh, |s| s.with(rate, reported as f64, now));
            m.insert(user.to_owned(), speed);
            speed
        };
        let user = user.to_owned();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO soulseek_speeds (user, rate, weight, reported, measured_at) VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(user) DO UPDATE SET rate = excluded.rate, weight = excluded.weight,
                       reported = excluded.reported, measured_at = excluded.measured_at",
                    params![user, speed.rate, speed.weight, speed.reported, speed.at],
                )?;
                Ok(())
            })
            .await
    }

    pub fn stalled_before(&self, user: &str) -> bool {
        let since = lull_core::now() - MEMORY_SECS;
        self.stalled
            .read()
            .is_ok_and(|m| m.get(user).is_some_and(|&at| at >= since))
    }

    pub async fn stalled(&self, user: &str) -> Result<()> {
        let now = lull_core::now();
        if let Ok(mut m) = self.stalled.write() {
            m.insert(user.to_owned(), now);
        }
        let user = user.to_owned();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO soulseek_peers (user, stalls, stalled_at) VALUES (?1, 1, ?2)
                     ON CONFLICT(user) DO UPDATE SET stalls = stalls + 1, stalled_at = excluded.stalled_at",
                    params![user, now],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delivered(&self, user: &str) -> Result<()> {
        let known = self.stalled.write().is_ok_and(|mut m| m.remove(user).is_some());
        if !known {
            return Ok(());
        }
        let user = user.to_owned();
        self.db
            .call(move |c| {
                c.execute("DELETE FROM soulseek_peers WHERE user = ?1", [user])?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 24 * 3600;

    #[test]
    fn a_peers_old_measurements_fade_so_how_it_sends_now_counts_most() {
        let fast = Speed {
            rate: 1_000_000.0,
            weight: 1.0,
            reported: 0.0,
            at: 0,
        };
        let a_week_on = fast.with(100_000.0, 0.0, 7 * DAY);
        assert!(
            (a_week_on.rate - 400_000.0).abs() < 1.0,
            "half-weighted old, full new: {}",
            a_week_on.rate
        );
        let same_day = fast.with(100_000.0, 0.0, 0);
        assert!(
            (same_day.rate - 550_000.0).abs() < 1.0,
            "equal weights the same day: {}",
            same_day.rate
        );
        assert!(
            fast.with(100_000.0, 0.0, 70 * DAY).rate < 101_000.0,
            "ten weeks on, the old one barely counts"
        );
    }

    #[tokio::test]
    async fn an_unmeasured_peer_is_ranked_by_its_report_scaled_as_reports_have_proved_here() -> Result<()> {
        let db = Db::memory()?;
        db.migrate(&crate::MIGRATIONS)?;
        let peers = Peers::load(db).await?;
        assert_eq!(
            peers.expected("new", 2_000_000),
            (2_000_000.0, None),
            "nothing measured: the report"
        );
        // Five peers that each sent a quarter of what they reported.
        for (i, reported) in [400_000u64, 800_000, 1_200_000, 1_600_000, 2_000_000]
            .into_iter()
            .enumerate()
        {
            peers.sent(&format!("p{i}"), reported as f64 / 4.0, reported).await?;
        }
        assert_eq!(
            peers.expected("new", 2_000_000).0,
            500_000.0,
            "the report, scaled to how reports proved"
        );
        let (rate, measured) = peers.expected("p0", 999_999_999);
        assert_eq!(rate, 100_000.0, "a measured peer: what it sent, whatever it reports");
        assert!(measured.is_some());
        Ok(())
    }
}
