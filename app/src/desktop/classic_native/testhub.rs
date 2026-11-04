//! A stand-in hub for the engine's tests: a small HTTP server on this machine that streams generated
//! songs (WAV, with ranges, whole or cut short, or refusing) and answers the calls the player makes,
//! keeping each for the test to look at.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// How a song's stream answers.
#[derive(Clone)]
pub enum Serve {
    /// The whole song, ranges honoured.
    Whole(Vec<u8>),
    /// The hub has no copy: 502 and its sentence.
    Refuse(String),
}

#[derive(Default)]
pub struct Book {
    pub songs: HashMap<String, Serve>,
    pub settings: Value,
    pub radio: Vec<String>,
    /// How many times each song's stream was asked for.
    pub fetches: HashMap<String, usize>,
    /// Better copies, by song: what `/api/stream/{id}/copy` streams.
    pub copies: HashMap<String, Vec<u8>>,
    /// This device listens privately: plays are answered with no id.
    pub private: bool,
    /// Calls to paths starting with these are dropped unanswered, as a hub out of reach would.
    pub unreachable: Vec<String>,
    /// Every call but the songs': method, path, body.
    pub calls: Vec<(String, String, Value)>,
}

pub struct TestHub {
    pub origin: String,
    pub book: Arc<Mutex<Book>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl TestHub {
    pub fn start(book: Book) -> Option<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").ok()?;
        let origin = format!("http://{}", listener.local_addr().ok()?);
        let book = Arc::new(Mutex::new(book));
        let shared = book.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let book = shared.clone();
                std::thread::spawn(move || answer(stream, &book));
            }
        });
        Some(Self { origin, book })
    }

    pub fn calls(&self, method: &str, path: &str) -> Vec<Value> {
        lock(&self.book)
            .calls
            .iter()
            .filter(|(m, p, _)| m == method && p.starts_with(path))
            .map(|(_, _, b)| b.clone())
            .collect()
    }
}

fn answer(stream: TcpStream, book: &Mutex<Book>) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let (method, target) = (
        parts.next().unwrap_or("").to_owned(),
        parts.next().unwrap_or("").to_owned(),
    );
    let mut headers = HashMap::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    let length: usize = headers.get("content-length").and_then(|l| l.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    let _ = reader.read_exact(&mut body);
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let path = target.split('?').next().unwrap_or("").to_owned();
    let mut out = stream;
    if let Some(id) = path.strip_prefix("/api/stream/").and_then(|p| p.strip_suffix("/copy"))
        && method == "GET"
    {
        let copy = lock(book).copies.get(id).cloned();
        return song(&mut out, copy.map(Serve::Whole), headers.get("range"));
    }
    if lock(book).unreachable.iter().any(|p| path.starts_with(p.as_str())) {
        return;
    }
    if let Some(id) = path
        .strip_prefix("/api/stream/")
        .or_else(|| path.strip_prefix("/api/library/stream/"))
        && method == "GET"
        && !id.contains('/')
    {
        let serve = {
            let mut book = lock(book);
            *book.fetches.entry(id.to_owned()).or_default() += 1;
            book.songs.get(id).cloned()
        };
        return song(&mut out, serve, headers.get("range"));
    }
    let reply = {
        let mut book = lock(book);
        book.calls.push((method.clone(), path.clone(), body));
        match (method.as_str(), path.as_str()) {
            ("GET", "/api/settings") => Some(book.settings.clone()),
            ("POST", "/api/plays") if book.private => Some(json!({ "playId": null, "private": true })),
            ("POST", "/api/plays") => Some(json!({ "playId": format!("p{}", book.calls.len()) })),
            ("GET", "/api/radio") => {
                let items = book.radio.clone();
                let tracks: serde_json::Map<String, Value> = items
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            json!({ "id": id, "title": id, "artist": "Radio", "saved": true, "duration": 2 }),
                        )
                    })
                    .collect();
                Some(json!({ "items": items, "entities": { "tracks": tracks } }))
            }
            _ => None,
        }
    };
    let _ = match reply {
        Some(v) => {
            let text = v.to_string();
            write!(
                out,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            )
        }
        None => write!(out, "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"),
    };
}

fn song(out: &mut TcpStream, serve: Option<Serve>, range: Option<&String>) {
    let bytes = match serve {
        Some(Serve::Whole(bytes)) => bytes,
        Some(Serve::Refuse(said)) => {
            let text = json!({ "error": "upstream", "message": said }).to_string();
            let _ = write!(
                out,
                "HTTP/1.1 502 Bad Gateway\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                text.len()
            );
            return;
        }
        None => {
            let _ = write!(
                out,
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            return;
        }
    };
    let from = range
        .and_then(|r| r.strip_prefix("bytes="))
        .and_then(|r| r.split('-').next())
        .and_then(|f| f.parse::<usize>().ok())
        .unwrap_or(0)
        .min(bytes.len());
    let head = if range.is_some() {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {from}-{}/{}\r\n",
            bytes.len().saturating_sub(1),
            bytes.len()
        )
    } else {
        "HTTP/1.1 200 OK\r\n".to_owned()
    };
    let _ = write!(
        out,
        "{head}Accept-Ranges: bytes\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len() - from
    );
    let _ = out.write_all(&bytes[from..]);
}

/// A WAV of music-like noise (no two stretches alike, its level swelling), `seconds` long, starting
/// `late` seconds into it: two of them with different `late` are two rips of one song.
pub fn noise(seconds: f64, late: f64) -> Vec<u8> {
    let rate = 48_000.0;
    let mut x: u32 = 0x9e37_79b9;
    let all: Vec<f64> = (0..((seconds + late + 1.0) * rate) as usize)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (f64::from(x) / f64::from(u32::MAX) - 0.5) * (0.6 + 0.4 * (i as f64 / 9_000.0).sin())
        })
        .collect();
    let skip = (late * rate) as usize;
    wav(&all[skip..skip + (seconds * rate) as usize])
}

/// A WAV of a tone: `seconds` long at 48 kHz, stereo, 16-bit, its level `level`.
pub fn tone(seconds: f64, hz: f64, level: f64) -> Vec<u8> {
    let samples: Vec<f64> = (0..(seconds * 48_000.0) as usize)
        .map(|i| (i as f64 / 48_000.0 * hz * std::f64::consts::TAU).sin() * level)
        .collect();
    wav(&samples)
}

/// Mono samples (-1 to 1) as a 48 kHz stereo 16-bit WAV.
fn wav(samples: &[f64]) -> Vec<u8> {
    let rate = 48_000u32;
    let frames = samples.len() as u32;
    let data = frames * 4;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let s = (sample * 32_000.0) as i16;
        wav.extend_from_slice(&s.to_le_bytes());
        wav.extend_from_slice(&s.to_le_bytes());
    }
    wav
}
