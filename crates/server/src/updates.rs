//! `GET /api/updates`: this hub's build, lull's newest release, and which of the two is newer. The newest
//! release is GitHub's "latest" for lull's repository, which leaves out drafts and pre-releases (slskd's
//! builds for the Windows app are pre-releases). Asked only when someone looks (Settings › App), kept for six
//! hours; `?check=true` asks again, at most once a minute.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use lull_core::Error;
use lull_core::build::{self, Build};
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::hub::Hub;

const LATEST: &str = "https://api.github.com/repos/TannhauserCommons/lull/releases/latest";
const KEEP: Duration = Duration::from_secs(6 * 3600);
const AGAIN: Duration = Duration::from_secs(60);

pub fn router() -> Router<Arc<Hub>> {
    Router::new().route("/api/updates", get(updates))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Latest {
    pub version: String,
    pub url: String,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Standing {
    /// The newest release, and nothing on top.
    Current,
    /// A newer release is out.
    Available,
    /// Everything in the newest release and changes since: it follows lull's main.
    Ahead,
    /// The newest release isn't known (GitHub didn't answer).
    Unknown,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    version: String,
    build: Build,
    latest: Option<Latest>,
    state: Standing,
    checked_at: Option<i64>,
    error: Option<String>,
}

#[derive(Clone)]
struct Checked {
    at: Instant,
    unix: i64,
    latest: Result<Latest, String>,
}

static LAST: Mutex<Option<Checked>> = Mutex::new(None);

#[derive(Deserialize)]
struct Ask {
    #[serde(default)]
    check: bool,
}

async fn updates(State(hub): State<Arc<Hub>>, Query(ask): Query<Ask>) -> Json<Report> {
    let last = LAST.lock().ok().and_then(|l| l.clone());
    let wait = if ask.check { AGAIN } else { KEEP };
    let checked = match last {
        Some(c) if c.at.elapsed() < wait => c,
        _ => {
            let ctx = hub.ctx().await;
            let c = Checked {
                at: Instant::now(),
                unix: lull_core::now(),
                latest: newest(&ctx.http).await.map_err(|e| e.to_string()),
            };
            if let Ok(mut l) = LAST.lock() {
                *l = Some(c.clone());
            }
            c
        }
    };
    let (latest, error) = match checked.latest {
        Ok(l) => (Some(l), None),
        Err(e) => (None, Some(e)),
    };
    Json(Report {
        version: build::version(),
        build: build::build(),
        state: standing(
            build::RELEASE,
            build::CHANGES,
            latest.as_ref().map(|l| l.version.as_str()),
        ),
        latest,
        checked_at: Some(checked.unix),
        error,
    })
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    published_at: Option<String>,
}

async fn newest(http: &lull_core::http::Http) -> lull_core::Result<Latest> {
    let r: Release = http
        .json("github", |c| {
            c.get(LATEST).header("Accept", "application/vnd.github+json")
        })
        .await?;
    let version = r.tag_name.trim().trim_start_matches('v').to_owned();
    if build::parse_release(&version).is_none() {
        return Err(Error::upstream(
            "github",
            format!("lull's newest release is named {:?}", r.tag_name),
        ));
    }
    Ok(Latest {
        version,
        url: r.html_url,
        published_at: r.published_at,
    })
}

/// Where a build stands against the newest release.
pub fn standing(release: &str, changes: u32, latest: Option<&str>) -> Standing {
    let (Some(have), Some(newest)) = (build::parse_release(release), latest.and_then(build::parse_release)) else {
        return Standing::Unknown;
    };
    if newest > have {
        Standing::Available
    } else if changes > 0 {
        Standing::Ahead
    } else {
        Standing::Current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_stands_against_the_newest_release() {
        assert_eq!(standing("0.2.4", 0, Some("0.2.4")), Standing::Current);
        assert_eq!(standing("0.2.4", 0, Some("0.2.5")), Standing::Available);
        assert_eq!(standing("0.2.4", 12, Some("0.2.4")), Standing::Ahead);
        // main from before 0.2.5: behind it, changes or not
        assert_eq!(standing("0.2.4", 12, Some("0.2.5")), Standing::Available);
        // a build newer than any release (the release commit landed before the release was published)
        assert_eq!(standing("0.2.5", 0, Some("0.2.4")), Standing::Current);
        assert_eq!(standing("0.2.4", 0, None), Standing::Unknown);
        assert_eq!(standing("0.2.4", 0, Some("nightly")), Standing::Unknown);
    }
}
