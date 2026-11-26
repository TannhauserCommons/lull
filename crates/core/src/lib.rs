//! Shared contracts for every lull crate. Domain crates depend on this and nothing else of
//! lull's; `server` wires them together. Changes here are additive only (AGENTS.md).

pub mod art;
pub mod build;
pub mod catalog;
pub mod config;
pub mod db;
pub mod devices;
pub mod error;
pub mod etag;
pub mod events;
pub mod flags;
pub mod helpers;
pub mod http;
pub mod identity;
pub mod ids;
pub mod m4a;
pub mod model;
pub mod pictures;
pub mod presets;
pub mod private;
pub mod process;
pub mod quality;
pub mod seams;
pub mod secrets;
pub mod settings;
pub mod status;
pub mod warm;
pub mod youtube;

use std::sync::Arc;

pub use error::{Error, Result};

/// What every crate gets at start-up.
#[derive(Clone)]
pub struct Ctx {
    pub config: Arc<config::Config>,
    pub db: db::Db,
    pub http: http::Http,
    pub bus: events::Bus,
    pub status: status::Status,
    /// Outside pictures (covers, artist photos), ordered by what the listener's screens show
    /// (ADR-0033). Built with `Ctx`'s own `http`, `bus` and `shutdown`.
    pub pictures: pictures::Pictures,
    /// Cancelled when the hub stops this set of crates: on exit, and before an in-process restart
    /// rewires them with new settings (ADR-0002). Background tasks end on it; see `Ctx::spawn`.
    pub shutdown: tokio_util::sync::CancellationToken,
}

impl Ctx {
    /// The context a set of crates starts with: its status and pictures report on `bus`, and
    /// everything it starts ends with its `shutdown`.
    pub fn new(config: config::Config, db: db::Db, http: http::Http, bus: events::Bus) -> Ctx {
        let shutdown = tokio_util::sync::CancellationToken::new();
        let pictures = pictures::Pictures::new(
            config.data_dir.join("catalog-art"),
            http.clone(),
            bus.clone(),
            shutdown.clone(),
        );
        Ctx {
            config: Arc::new(config),
            db,
            http,
            status: status::Status::new(bus.clone()),
            bus,
            pictures,
            shutdown,
        }
    }

    /// Runs a background task until it finishes or the hub stops this set of crates, so a
    /// restart never leaves the old crates' loops running beside the new ones.
    pub fn spawn<F>(&self, task: F)
    where
        F: std::future::Future + Send + 'static,
        F::Output: Send,
    {
        let shutdown = self.shutdown.clone();
        tokio::spawn(async move {
            shutdown.run_until_cancelled(task).await;
        });
    }
}

/// Unix seconds now.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Unix seconds as an ISO 8601 UTC time, `2026-11-01T12:00:00Z`: its text sorts in time order.
pub fn iso_utc(unix: i64) -> String {
    let secs = unix.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        iso_date(unix),
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Unix seconds as the UTC date, `2026-11-01`. A local day is the date of the time plus its UTC offset.
pub fn iso_date(unix: i64) -> String {
    let (y, m, d) = civil_from_days(unix.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian (year, month, day), by Howard Hinnant's civil-from-days.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::{iso_date, iso_utc};

    #[test]
    fn unix_seconds_read_as_the_gregorian_calendar() {
        for (unix, utc) in [
            (0, "1970-01-01T00:00:00Z"),
            (-1, "1969-12-31T23:59:59Z"),
            (946_684_799, "1999-12-31T23:59:59Z"),
            (946_684_800, "2000-01-01T00:00:00Z"),
            // Leap years: every fourth, but a century only when it is also a fourth century.
            (951_782_400, "2000-02-29T00:00:00Z"),
            (951_868_800, "2000-03-01T00:00:00Z"),
            (-2_203_891_201, "1900-02-28T23:59:59Z"),
            (-2_203_891_200, "1900-03-01T00:00:00Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"),
            (-11_670_912_001, "1600-02-29T23:59:59Z"),
            (1_793_534_400, "2026-11-01T12:00:00Z"),
        ] {
            assert_eq!(iso_utc(unix), utc, "{unix}");
            assert_eq!(iso_date(unix), utc[..10], "{unix}");
        }
    }
}
