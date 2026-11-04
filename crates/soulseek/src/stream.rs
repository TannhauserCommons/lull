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
/// Re-checks the file this often while waiting for slskd to write more.
const TICK: Duration = Duration::from_millis(250);

pub async fn serve(fetch: Arc<Fetch>, headers: &HeaderMap) -> Result<Response> {
    let mut rx = fetch.progress();
    let requested = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (ask, span, p) = match ready(&mut rx, requested).await? {
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
async fn ready(rx: &mut watch::Receiver<Progress>, requested: Option<&str>) -> Result<Ready> {
    loop {
        // The source decides the size and type; it may change through fallbacks until bytes flow.
        let first = wait(rx, |p| p.source.is_some() && p.path.is_some()).await?;
        let ask = range::ask(requested, first.size());
        let span = match ask {
            Ask::Whole(s) | Ask::Part(s) => s,
            Ask::Unsatisfiable => return Ok(Ready::Unsatisfiable(first.size())),
        };
        let p = wait(rx, |p| p.path.is_some() && p.received > span.start).await?;
        if p.received <= span.start && span.len() > 0 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::Status;
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
            received,
            speed: 0,
            queue_pos: None,
            path: Some("song.mp3".into()),
            error: None,
            failed_peers: Vec::new(),
            peers_found: 1,
            below: false,
            choices: Vec::new(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_seek_ahead_of_the_transfer_waits_for_its_bytes_however_long_they_take() {
        let (tx, mut rx) = watch::channel(arriving(1000, 100));
        let answer = tokio::spawn(async move { ready(&mut rx, Some("bytes=500-")).await });
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert!(!answer.is_finished(), "no 416 while the transfer is on its way there");
        tx.send_modify(|p| p.received = 501);
        let served = answer.await;
        assert!(
            matches!(&served, Ok(Ok(Ready::Serve(Ask::Part(Span { start: 500, end: 1000 }), _, p))) if p.received == 501),
            "the range is served once its bytes are here: {served:?}"
        );
    }

    #[tokio::test]
    async fn only_a_range_past_the_end_of_the_file_is_unsatisfiable() {
        let (_tx, mut rx) = watch::channel(arriving(1000, 100));
        assert!(matches!(
            ready(&mut rx, Some("bytes=1000-")).await,
            Ok(Ready::Unsatisfiable(1000))
        ));
        let (tx, mut rx) = watch::channel(arriving(1000, 100));
        tx.send_modify(|p| p.status = Status::Done);
        assert!(
            matches!(ready(&mut rx, Some("bytes=500-")).await, Ok(Ready::Unsatisfiable(1000))),
            "a finished file shorter than the peer said can't serve the rest"
        );
    }
}
