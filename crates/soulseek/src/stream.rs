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

use crate::fetch::{Fetch, Progress, Reader};
use crate::range::{self, Ask, Span};

const CHUNK: usize = 64 * 1024;
/// A seek past what has arrived waits this long for the bytes before answering 416.
const SEEK_WAIT: Duration = Duration::from_secs(8);
/// Re-checks the file this often while waiting for slskd to write more.
const TICK: Duration = Duration::from_millis(250);

pub async fn serve(fetch: Arc<Fetch>, headers: &HeaderMap) -> Result<Response> {
    let mut rx = fetch.progress();
    // The source decides the size and type; it may change through fallbacks until bytes flow.
    let p = wait(&mut rx, |p| p.source.is_some() && p.path.is_some(), None).await?;
    let requested = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let ask = range::ask(requested, p.size());
    let span = match ask {
        Ask::Whole(s) | Ask::Part(s) => s,
        Ask::Unsatisfiable => return Ok(unsatisfiable(p.size())),
    };
    let seeking = span.start > 0;
    let p = wait(
        &mut rx,
        |p| p.path.is_some() && p.received > span.start,
        seeking.then_some(SEEK_WAIT),
    )
    .await;
    let p = match p {
        Ok(p) if p.received > span.start || span.len() == 0 => p,
        // The bytes didn't arrive in time, or the fetch ended without them.
        Ok(p) if p.status == crate::fetch::Status::Failed => return Err(failed(&p)),
        _ => return Ok(unsatisfiable(p_size(&rx))),
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

/// Waits until `ready` holds, the fetch ends, or `limit` passes (then errs).
async fn wait(
    rx: &mut watch::Receiver<Progress>,
    ready: impl Fn(&Progress) -> bool,
    limit: Option<Duration>,
) -> Result<Progress> {
    let waiting = rx.wait_for(|p| ready(p) || p.ended());
    let p = match limit {
        Some(limit) => tokio::time::timeout(limit, waiting)
            .await
            .map_err(|_| Error::Conflict("those bytes have not arrived yet".into()))?,
        None => waiting.await,
    }
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

fn p_size(rx: &watch::Receiver<Progress>) -> u64 {
    rx.borrow().size()
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
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        _ => "application/octet-stream",
    }
}

struct Feed {
    /// Keeps the fetch counted as playing while the body is in flight.
    _reader: Reader,
    rx: watch::Receiver<Progress>,
    attempt: u32,
    span: Span,
    pos: u64,
    path: std::path::PathBuf,
    file: Option<File>,
}

/// The body: read what's on disk, wait for more, stop at the span's end. The file's own
/// length is the truth for what has arrived; slskd writes it front to back and never sizes it
/// up front. The file is reopened by path whenever the fetch sees more on disk than this
/// handle does: slskd moves the finished file, and may recreate it when it retries a peer.
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
                match read_chunk(f, s.span, s.pos).await {
                    Ok(buf) if !buf.is_empty() => {
                        s.pos += buf.len() as u64;
                        return Ok(Some((Bytes::from(buf), s)));
                    }
                    // Nothing new through this handle, but the fetch sees more at its path (or a
                    // new path): the file was moved or recreated under us.
                    Ok(_) if p.received > s.pos || p.path.as_ref() != Some(&s.path) => s.file = None,
                    Ok(_) if p.ended() => {
                        return Err(std::io::Error::other(match p.status {
                            crate::fetch::Status::Done => "the file is shorter than the peer said",
                            _ => "the transfer ended before this part arrived",
                        }));
                    }
                    Ok(_) => {}
                    // The handle went bad while slskd is still writing: reopen by path.
                    Err(_) if !p.ended() => s.file = None,
                    Err(e) => return Err(e),
                }
            }
            let _ = tokio::time::timeout(TICK, s.rx.changed()).await;
        }
    })
}

/// Up to one chunk of the span from `pos`, never past what this handle sees on disk.
async fn read_chunk(f: &mut File, span: Span, pos: u64) -> std::io::Result<Vec<u8>> {
    let on_disk = f.metadata().await?.len();
    let want = range::sendable(span, pos, on_disk).min(CHUNK as u64) as usize;
    let mut buf = vec![0; want];
    let n = if want > 0 { f.read(&mut buf).await? } else { 0 };
    buf.truncate(n);
    Ok(buf)
}
