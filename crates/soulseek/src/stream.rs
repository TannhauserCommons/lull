//! `GET /api/stream/{trackId}`: audio straight from the file slskd is writing. Bytes are read
//! from disk in small chunks as they land and never beyond what's there, so memory stays
//! bounded and nothing is copied twice. A fallback to another peer ends the response; the
//! player reopens the stream (docs/context/streaming.md#serving-bytes).

use std::io::SeekFrom;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lull_core::{Error, Result};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::watch;

use crate::fetch::{Fetch, Progress, Reader, Waiting};
use crate::range::{self, Ask, Span};

const CHUNK: usize = 64 * 1024;
/// Re-checks the file this often while waiting for slskd to write more.
const TICK: Duration = Duration::from_millis(250);

pub async fn serve(fetch: Arc<Fetch>, headers: &HeaderMap) -> Result<Response> {
    let mut rx = fetch.progress();
    let requested = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (ask, span, p) = match ready(&mut rx, requested, |byte| fetch.ask_for(byte)).await? {
        Ready::Serve(ask, span, p) => (ask, span, *p),
        Ready::Unsatisfiable(size) => return Ok(unsatisfiable(size)),
    };
    let source = p
        .source
        .as_ref()
        .ok_or_else(|| Error::internal("stream without a source"))?;
    let path = p.path.clone().ok_or_else(|| Error::internal("stream without a file"))?;

    let mut resp = Response::builder()
        .header(header::CONTENT_TYPE, content_type(&source.ext))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, span.len())
        .header(header::CACHE_CONTROL, "no-store");
    resp = match ask {
        Ask::Part(s) => resp
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, s.content_range(p.size())),
        _ => resp.status(StatusCode::OK),
    };
    let body = Body::from_stream(chunks(Feed {
        _reader: fetch.reading(),
        fetch: fetch.clone(),
        asked: None,
        rx,
        attempt: p.attempt,
        span,
        pos: span.start,
        path,
        file: None,
    }));
    resp.body(body)
        .map_err(|e| Error::internal(format!("stream response: {e}")))
}

/// How a request is answered once the fetch can say.
#[derive(Debug)]
enum Ready {
    Serve(Ask, Span, Box<Progress>),
    /// The range starts past the end of the file (of this size).
    Unsatisfiable(u64),
}

/// Waits until the fetch has a source and bytes cover the requested start. A range inside the
/// file is satisfiable, only not yet: it waits for its bytes as long as the transfer runs, as a
/// request from byte 0 does, so a seek ahead of the transfer plays once it gets there instead of
/// failing. Only a range past the end of the file is 416.
async fn ready<A>(
    rx: &mut watch::Receiver<Progress>,
    requested: Option<&str>,
    ask_for: impl Fn(u64) -> A,
) -> Result<Ready> {
    loop {
        // The source decides the size and type; it may change through fallbacks until bytes flow.
        let first = wait(rx, |p| p.source.is_some() && p.path.is_some()).await?;
        let ask = range::ask(requested, first.size());
        let span = match ask {
            Ask::Whole(s) | Ask::Part(s) => s,
            Ask::Unsatisfiable => return Ok(Ready::Unsatisfiable(first.size())),
        };
        // A start that hasn't arrived is asked for while this waits: the fetch may start a leg
        // there.
        let _asked = (span.len() > 0 && !first.ranges.has(span.start)).then(|| ask_for(span.start));
        let p = wait(rx, |p| p.path.is_some() && p.ranges.has(span.start)).await?;
        if !p.ranges.has(span.start) && span.len() > 0 {
            // The fetch ended without those bytes: it failed, or the file is shorter than the
            // peer said.
            return match p.status {
                crate::fetch::Status::Failed => Err(failed(&p)),
                _ => Ok(Ready::Unsatisfiable(p.size())),
            };
        }
        // Another source took over in between (a peer fell back, YouTube stepped in): its size
        // and type may differ, so the answer is worked out again.
        if p.attempt == first.attempt {
            return Ok(Ready::Serve(ask, span, Box::new(p)));
        }
    }
}

/// Waits until `ready` holds or the fetch ends.
async fn wait(rx: &mut watch::Receiver<Progress>, ready: impl Fn(&Progress) -> bool) -> Result<Progress> {
    let p = rx
        .wait_for(|p| ready(p) || p.ended())
        .await
        .map(|p| p.clone())
        .map_err(|_| Error::internal("the transfer went away"))?;
    match p.status {
        crate::fetch::Status::Failed if p.path.is_none() => Err(failed(&p)),
        crate::fetch::Status::Cancelled => Err(Error::Conflict("the stream was stopped".into())),
        _ => Ok(p),
    }
}

fn failed(p: &Progress) -> Error {
    Error::upstream(
        "soulseek",
        p.error.clone().unwrap_or_else(|| "no peer delivered the song".into()),
    )
}

fn unsatisfiable(size: u64) -> Response {
    let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    if let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}")) {
        r.headers_mut().insert(header::CONTENT_RANGE, v);
    }
    r
}

fn content_type(ext: &str) -> &'static str {
    match ext {
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "m4a" | "aac" => "audio/mp4",
        // YouTube's Opus stream, as YouTube serves it.
        "webm" => "audio/webm",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

struct Feed {
    /// Keeps the fetch counted as playing while the body is in flight.
    _reader: Reader,
    /// Asked for the byte the body waits for at a gap, while it waits there.
    fetch: Arc<Fetch>,
    asked: Option<(u64, Waiting)>,
    rx: watch::Receiver<Progress>,
    attempt: u32,
    span: Span,
    pos: u64,
    path: std::path::PathBuf,
    file: Option<File>,
}

/// The body: read what has arrived, wait at a gap for more, stop at the span's end. The fetch's
/// ranges are the truth for what has arrived (a copy arrives in legs, ADR-0014), not the file's
/// length. The file is reopened when the fetch's path changes, or when its ranges hold bytes
/// this handle doesn't see (a YouTube copy's file recreated under us).
fn chunks(state: Feed) -> impl futures_util::Stream<Item = std::io::Result<Bytes>> {
    futures_util::stream::try_unfold(state, |mut s| async move {
        loop {
            if s.pos >= s.span.end {
                return Ok(None);
            }
            let p = s.rx.borrow().clone();
            if p.attempt != s.attempt {
                return Err(std::io::Error::other("the stream moved to another peer; reopen it"));
            }
            if s.file.is_none()
                && let Some(path) = p.path.clone()
            {
                match File::open(&path).await {
                    Ok(mut f) => {
                        f.seek(SeekFrom::Start(s.pos)).await?;
                        s.file = Some(f);
                        s.path = path;
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound && !p.ended() => {}
                    Err(e) => return Err(e),
                }
            }
            if let Some(f) = s.file.as_mut() {
                let arrived = p.ranges.end_from(s.pos).unwrap_or(s.pos);
                match read_chunk(f, s.span, s.pos, arrived).await {
                    Ok(buf) if !buf.is_empty() => {
                        s.pos += buf.len() as u64;
                        s.asked = None;
                        return Ok(Some((Bytes::from(buf), s)));
                    }
                    // Nothing new through this handle, but the fetch sees more at its path (or a
                    // new path): the file was moved or recreated under us.
                    Ok(_) if arrived > s.pos || p.path.as_ref() != Some(&s.path) => s.file = None,
                    Ok(_) if p.ended() => {
                        return Err(std::io::Error::other(match p.status {
                            crate::fetch::Status::Done => "the file is shorter than the peer said",
                            _ => "the transfer ended before this part arrived",
                        }));
                    }
                    // At a gap, with the transfer still on: the byte is asked for, so a leg may
                    // start there rather than this body waiting for the one running to arrive.
                    Ok(_) => {
                        if s.asked.as_ref().is_none_or(|(at, _)| *at != s.pos) {
                            s.asked = Some((s.pos, s.fetch.ask_for(s.pos)));
                        }
                    }
                    // The handle went bad while slskd is still writing: reopen by path.
                    Err(_) if !p.ended() => s.file = None,
                    Err(e) => return Err(e),
                }
            }
            let _ = tokio::time::timeout(TICK, s.rx.changed()).await;
        }
    })
}

/// Up to one chunk of the span from `pos`, never past `arrived`, the end of the bytes that have
/// arrived from `pos` on.
async fn read_chunk(f: &mut File, span: Span, pos: u64, arrived: u64) -> std::io::Result<Vec<u8>> {
    let want = range::sendable(span, pos, arrived).min(CHUNK as u64) as usize;
    let mut buf = vec![0; want];
    let n = if want > 0 { f.read(&mut buf).await? } else { 0 };
    buf.truncate(n);
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Status;
    use crate::ranges::Ranges;
    use crate::sources::Source;

    fn arriving(size: u64, received: u64) -> Progress {
        Progress {
            attempt: 0,
            status: Status::Streaming,
            source: Some(Source {
                user: "peer".into(),
                path: "Music\\A\\01 - Song.mp3".into(),
                size,
                ext: "mp3".into(),
                facts: lull_core::model::FileFacts {
                    format: "MP3 320".into(),
                    codec: "MP3".into(),
                    lossless: false,
                    khz: Some(44.1),
                    bits: None,
                    kbps: Some(320),
                    size_bytes: Some(size),
                },
                upload_speed: 0,
                free_slot: true,
                queue: 0,
                length: Some(200),
            }),
            ranges: Ranges::front(received),
            speed: 0,
            queue_pos: None,
            path: Some("song.mp3".into()),
            error: None,
            failed_peers: Vec::new(),
            peers_found: 1,
            below: false,
            choices: Vec::new(),
            ahead: None,
            unverified: false,
            unplayable: None,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_seek_ahead_of_the_transfer_waits_for_its_bytes_however_long_they_take() {
        let (tx, mut rx) = watch::channel(arriving(1000, 100));
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let asks = asked.clone();
        let answer = tokio::spawn(async move {
            ready(&mut rx, Some("bytes=500-"), |byte| {
                if let Ok(mut a) = asks.lock() {
                    a.push(byte);
                }
            })
            .await
        });
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert!(!answer.is_finished(), "no 416 while the transfer is on its way there");
        assert_eq!(
            *asked.lock().unwrap_or_else(|e| e.into_inner()),
            [500],
            "the byte is asked for, so a leg may start there"
        );
        tx.send_modify(|p| p.ranges.add(0, 501));
        let served = answer.await;
        assert!(
            matches!(&served, Ok(Ok(Ready::Serve(Ask::Part(Span { start: 500, end: 1000 }), _, p))) if p.received() == 501),
            "the range is served once its bytes are here: {served:?}"
        );
    }

    #[tokio::test]
    async fn only_a_range_past_the_end_of_the_file_is_unsatisfiable() {
        let (_tx, mut rx) = watch::channel(arriving(1000, 100));
        assert!(matches!(
            ready(&mut rx, Some("bytes=1000-"), |_| {}).await,
            Ok(Ready::Unsatisfiable(1000))
        ));
        let (tx, mut rx) = watch::channel(arriving(1000, 100));
        tx.send_modify(|p| p.status = Status::Done);
        assert!(
            matches!(
                ready(&mut rx, Some("bytes=500-"), |_| {}).await,
                Ok(Ready::Unsatisfiable(1000))
            ),
            "a finished file shorter than the peer said can't serve the rest"
        );
    }
}
