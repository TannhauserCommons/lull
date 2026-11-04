//! A song on the hub as a file Symphonia can read and seek in, by the hub's streaming rules
//! (docs/context/api.md, `GET /api/stream/{id}`; crates/soulseek/src/stream.rs): the answer waits
//! for the first bytes for as long as the hub looks for a copy, so only connecting has a time limit;
//! `Content-Length` is the whole song's, not what has arrived, and the body trickles in as it does;
//! a range past what has arrived waits 8 s and then answers 416; a body that ends early or breaks
//! means the hub moved to another copy or the transfer stopped (the `stream` events say which). A
//! seek close ahead reads on; further, it asks for the rest from there (a `Range` request).

use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;
use symphonia::core::io::MediaSource;
use ureq::tls::{RootCerts, TlsConfig};

/// A seek this close ahead reads on through the open response instead of asking again.
const SKIP_READ: u64 = 64 * 1024;
/// The marker in an I/O error for a range past what has arrived (the hub's 416).
pub const NOT_ARRIVED: &str = "not arrived yet";

/// Why a song couldn't be opened.
#[derive(Clone, Debug, PartialEq)]
pub enum Refused {
    /// No answer from the hub.
    Offline(String),
    /// The hub answered with this status and sentence (502 no peer delivered, 409 stopped, 404
    /// unknown or not saved).
    Status(u16, String),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Offline(why) => write!(f, "can't reach the hub: {why}"),
            Refused::Status(_, said) => f.write_str(said),
        }
    }
}

/// An agent for songs: connecting has a limit, waiting for the answer and the body don't (the hub
/// answers once a copy's first bytes are here, which can take as long as finding one).
pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

pub struct Remote {
    agent: ureq::Agent,
    url: String,
    token: String,
    length: Option<u64>,
    position: u64,
    body: Option<ureq::BodyReader<'static>>,
}

impl Remote {
    /// Opens the song from `from` bytes in (0: the whole of it).
    pub fn open(agent: ureq::Agent, url: &str, token: &str, from: u64) -> Result<Self, Refused> {
        let mut remote = Self {
            agent,
            url: url.to_owned(),
            token: token.to_owned(),
            length: None,
            position: from,
            body: None,
        };
        remote.request()?;
        Ok(remote)
    }

    pub fn length(&self) -> Option<u64> {
        self.length
    }

    /// Asks for the song from `position` on.
    fn request(&mut self) -> Result<(), Refused> {
        let mut request = self
            .agent
            .get(&self.url)
            .header("Authorization", format!("Bearer {}", self.token));
        if self.position > 0 {
            request = request.header("Range", format!("bytes={}-", self.position));
        }
        let response = request.call().map_err(|e| Refused::Offline(e.to_string()))?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        if !(200..300).contains(&status) {
            if status == 416 {
                return Err(Refused::Status(416, NOT_ARRIVED.into()));
            }
            let mut answer = Vec::new();
            let _ = response
                .into_body()
                .into_reader()
                .take(64 * 1024)
                .read_to_end(&mut answer);
            let said = serde_json::from_slice::<serde_json::Value>(&answer)
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("the hub said {status}"));
            return Err(Refused::Status(status, said));
        }
        let partial = status == 206;
        if self.position > 0 && !partial {
            return Err(Refused::Status(status, "the hub can't seek in this song".into()));
        }
        if self.length.is_none() {
            self.length = if partial {
                header("content-range").and_then(|r| r.rsplit('/').next()?.parse().ok())
            } else {
                header("content-length").and_then(|l| l.parse().ok())
            };
        }
        self.body = Some(response.into_body().into_reader());
        Ok(())
    }
}

impl Read for Remote {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.body.is_none() {
            self.request().map_err(|e| match e {
                Refused::Status(416, _) => io::Error::new(io::ErrorKind::WouldBlock, NOT_ARRIVED),
                other => io::Error::other(other.to_string()),
            })?;
        }
        let read = match self.body.as_mut() {
            Some(body) => body.read(buf)?,
            None => 0,
        };
        // The body ended before the song did: the hub cut it (another copy, or the transfer ended).
        if read == 0 && !buf.is_empty() && self.length.is_some_and(|l| self.position < l) {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the hub ended the song's response early",
            ));
        }
        self.position += read as u64;
        Ok(read)
    }
}

impl Seek for Remote {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(at) => Some(at),
            SeekFrom::Current(by) => self.position.checked_add_signed(by),
            SeekFrom::End(by) => self.length.and_then(|l| l.checked_add_signed(by)),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek outside the song"))?;
        if target == self.position {
            return Ok(target);
        }
        if target > self.position && target - self.position <= SKIP_READ && self.body.is_some() {
            let skip = target - self.position;
            io::copy(&mut self.by_ref().take(skip), &mut io::sink())?;
            return Ok(self.position);
        }
        self.body = None;
        self.position = target;
        Ok(target)
    }
}

impl MediaSource for Remote {
    fn is_seekable(&self) -> bool {
        self.length.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.length
    }
}
