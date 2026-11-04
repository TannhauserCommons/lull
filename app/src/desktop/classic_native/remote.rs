//! A song on the hub as a file Symphonia can read and seek in, by the hub's streaming rules
//! (docs/context/api.md, `GET /api/stream/{id}`; crates/soulseek/src/stream.rs): the answer waits
//! for the first bytes for as long as the hub looks for a copy, so only connecting has a time limit;
//! `Content-Length` is the whole song's, not what has arrived, and the body trickles in as it does;
//! a range past what has arrived waits 8 s and then answers 416; a body that ends early or breaks
//! means the hub moved to another copy or the transfer stopped (the `stream` events say which). A
//! seek close ahead reads on; further, it asks for the rest from there (a `Range` request).
//!
//! A FLAC file's cover pictures pass as padding (`Pictures`): Symphonia would otherwise keep each
//! one, often a megabyte or more, for as long as the song is open, and nothing here shows it.

use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;
use symphonia::core::io::MediaSource;
use ureq::tls::{RootCerts, TlsConfig};

/// A seek this close ahead reads on through the open response instead of asking again.
const SKIP_READ: u64 = 64 * 1024;
/// Each connection's buffers (ureq's default is 128 KB each way): the hub's answers and their
/// headers fit in the one, its requests in the other, and a song arrives at a few hundred KB a
/// second at most, read on by Symphonia's own buffer. Every connection Classic holds open (the
/// songs playing and next, the events, the calls) would otherwise keep 256 KB.
pub const INPUT_BUFFER: usize = 32 * 1024;
pub const OUTPUT_BUFFER: usize = 16 * 1024;
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
        .input_buffer_size(INPUT_BUFFER)
        .output_buffer_size(OUTPUT_BUFFER)
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
    pictures: Pictures,
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
            pictures: Pictures::default(),
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
        self.pictures.pass(self.position, &mut buf[..read]);
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
    /// Not while a FLAC file's metadata passes: Symphonia then reads the padding a picture became
    /// instead of seeking past it, which on a song still arriving would ask for bytes that may not
    /// be here yet.
    fn is_seekable(&self) -> bool {
        self.length.is_some() && self.pictures.passed()
    }

    fn byte_len(&self) -> Option<u64> {
        self.length
    }
}

/// FLAC's metadata blocks as the song's first bytes pass: each PICTURE block's type byte becomes
/// PADDING's, so Symphonia skips the picture rather than reading it in (FLAC format: "fLaC", then
/// blocks of a 4-byte header, last-block flag and 7-bit type, then a 24-bit length, then the
/// body). Anything but a FLAC file read from its start passes untouched.
#[derive(Debug, Default)]
struct Pictures {
    state: Blocks,
}

#[derive(Debug)]
enum Blocks {
    /// This many bytes of the file's start have passed, all of them "fLaC" so far.
    Marker(usize),
    /// The next block's header starts at this byte of the file; `have` of its bytes seen.
    Header { at: u64, got: [u8; 4], have: usize },
    /// The audio has begun, or this isn't FLAC from its start.
    Passed,
}

impl Default for Blocks {
    fn default() -> Self {
        Blocks::Marker(0)
    }
}

const MARKER: &[u8; 4] = b"fLaC";
const PICTURE: u8 = 6;
const PADDING: u8 = 1;

impl Pictures {
    /// `bytes` of the file, starting at byte `at`, about to be handed on.
    fn pass(&mut self, at: u64, bytes: &mut [u8]) {
        if bytes.is_empty() {
            return;
        }
        let end = at + bytes.len() as u64;
        while let Blocks::Marker(seen) = self.state {
            if at > seen as u64 || seen as u64 >= end {
                if at > seen as u64 {
                    self.state = Blocks::Passed;
                }
                break;
            }
            self.state = match bytes[(seen as u64 - at) as usize] == MARKER[seen] {
                false => Blocks::Passed,
                true if seen + 1 == MARKER.len() => Blocks::Header {
                    at: MARKER.len() as u64,
                    got: [0; 4],
                    have: 0,
                },
                true => Blocks::Marker(seen + 1),
            };
        }
        while let Blocks::Header {
            at: header,
            mut got,
            mut have,
        } = self.state
        {
            let first = header + have as u64;
            if first < at {
                // Bytes of the header went by unseen (a seek): stop looking.
                self.state = Blocks::Passed;
                return;
            }
            if first >= end {
                return;
            }
            while have < 4 && header + (have as u64) < end {
                let i = (header + have as u64 - at) as usize;
                got[have] = bytes[i];
                if have == 0 && bytes[i] & 0x7f == PICTURE {
                    bytes[i] = bytes[i] & 0x80 | PADDING;
                }
                have += 1;
            }
            self.state = if have < 4 {
                Blocks::Header { at: header, got, have }
            } else if got[0] & 0x80 != 0 {
                Blocks::Passed
            } else {
                let length = u64::from(got[1]) << 16 | u64::from(got[2]) << 8 | u64::from(got[3]);
                Blocks::Header {
                    at: header + 4 + length,
                    got: [0; 4],
                    have: 0,
                }
            };
        }
    }

    /// The metadata has gone by (or there was none to watch).
    fn passed(&self) -> bool {
        matches!(self.state, Blocks::Passed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A FLAC file's start: STREAMINFO, a picture, a comment and a last picture, then audio.
    fn flac() -> (Vec<u8>, Vec<u8>) {
        let block = |kind: u8, body: &[u8]| {
            let mut b = vec![kind, 0, 0, body.len() as u8];
            b.extend_from_slice(body);
            b
        };
        let parts = [
            block(0, &[7; 34]),
            block(PICTURE, &[6; 20]),
            block(4, &[4; 9]),
            block(0x80 | PICTURE, &[6; 5]),
        ];
        let mut file = b"fLaC".to_vec();
        let mut expected = file.clone();
        for part in &parts {
            file.extend_from_slice(part);
            let mut seen = part.clone();
            if seen[0] & 0x7f == PICTURE {
                seen[0] = seen[0] & 0x80 | PADDING;
            }
            expected.extend_from_slice(&seen);
        }
        // Audio: a frame header and bytes that look like block headers but aren't.
        let audio = [0xff, 0xf8, PICTURE, 0, 0, 3, PICTURE];
        file.extend_from_slice(&audio);
        expected.extend_from_slice(&audio);
        (file, expected)
    }

    #[test]
    fn a_flac_files_pictures_pass_as_padding_however_its_bytes_arrive() {
        let (file, expected) = flac();
        for chunk in 1..=file.len() {
            let mut pictures = Pictures::default();
            let mut out = Vec::new();
            for (n, piece) in file.chunks(chunk).enumerate() {
                let mut piece = piece.to_vec();
                pictures.pass((n * chunk) as u64, &mut piece);
                out.extend_from_slice(&piece);
            }
            assert_eq!(out, expected, "read {chunk} bytes at a time");
            assert!(pictures.passed());
        }
    }

    #[test]
    fn other_songs_and_later_reads_pass_untouched() {
        let mut pictures = Pictures::default();
        let mut mp3 = b"ID3\x04\x00\x06\x06\x06".to_vec();
        pictures.pass(0, &mut mp3);
        assert_eq!(mp3, b"ID3\x04\x00\x06\x06\x06");
        assert!(pictures.passed());
        let (mut file, _) = flac();
        let mut later = Pictures::default();
        later.pass(4, &mut file[4..]);
        assert!(later.passed(), "a response from further in is past the metadata");
        assert_eq!(file[4 + 38], PICTURE, "and leaves its bytes alone");
    }
}
