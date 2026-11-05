//! Is this copy the recording? Its sound against a reference (ADR-0016): both are decoded
//! (symphonia: AAC/M4A, MP3, FLAC, Vorbis, ALAC, WAV; ruopus: YouTube's Opus), fingerprinted (rusty-chromaprint,
//! Chromaprint's algorithm 2) and the reference slid over every offset of the copy for the one with
//! the fewest differing bits. That bit error rate is the score: 0 is the same audio, about 0.5
//! unrelated.
//!
//! Measured on 281 hand-checked YouTube uploads of 100 songs (2026-11-05): the song's own
//! uploads score about 0.04, remasters and radio edits of it up to 0.16, other recordings
//! (other artists' songs of the title, covers, live, sped-up, slowed, re-recordings) 0.26 and
//! more. A track with the vocal removed sounds the same to it (instrumentals and karaoke as low
//! as 0.007), so the words that name those still refuse (`matching`).

use std::path::Path;

use rusty_chromaprint::{Configuration, Fingerprinter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_OPUS, DecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Under this the copy is the recording.
pub const SAME: f64 = 0.15;
/// From `SAME` to this, another master or mix of it: taken when the words agree too. At this and
/// over, another recording.
pub const OTHER: f64 = 0.25;
/// The reference must overlap the copy this long to count.
const MIN_OVERLAP_S: f64 = 15.0;

/// What a copy's sound says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Same,
    Near,
    Other,
}

impl Zone {
    pub fn of(score: f64) -> Zone {
        if score < SAME {
            Zone::Same
        } else if score < OTHER {
            Zone::Near
        } else {
            Zone::Other
        }
    }
}

/// A fingerprint and how much audio it covers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Print {
    pub items: Vec<u32>,
    pub seconds: f64,
}

/// The best alignment of a reference in a copy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Score {
    /// Bit error rate at the best offset.
    pub score: f64,
    /// Where the reference starts in the copy (seconds; negative: before it).
    pub offset_s: f64,
    pub overlap_s: f64,
}

fn config() -> Configuration {
    Configuration::preset_test2()
}

/// Seconds one fingerprint item covers.
pub fn item_s() -> f64 {
    f64::from(config().item_duration_in_seconds())
}

/// Decodes `path` and fingerprints it, all of it or its first `limit_s` seconds. An error names
/// what can't be read (an Opus file, a broken one).
pub fn print_file(path: &Path, limit_s: Option<f64>) -> Result<Print, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("can't open {}: {e}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("not audio lull can read: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no audio in the file")?.clone();
    let channels = track.codec_params.channels.map_or(2, |c| c.count()) as u32;
    let mut printer = Fingerprinter::new(&config());
    if track.codec_params.codec == CODEC_TYPE_OPUS {
        return opus(format, track.id, channels, printer, limit_s);
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("a codec lull can't decode: {e}"))?;
    let rate = track.codec_params.sample_rate.ok_or("no sample rate")?;
    printer.start(rate, channels).map_err(|e| format!("{e:?}"))?;
    let limit = limit_s.map(|s| (s * f64::from(rate)) as u64);
    let mut frames = 0u64;
    let mut buf: Option<SampleBuffer<i16>> = None;
    while limit.is_none_or(|l| frames < l) {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // The end, or a file still growing: what has arrived is what there is.
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(DecodeError::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        let samples = decoded.capacity() * spec.channels.count();
        let b = match &mut buf {
            Some(b) if b.capacity() >= samples => b,
            _ => buf.insert(SampleBuffer::new(decoded.capacity() as u64, spec)),
        };
        b.copy_interleaved_ref(decoded);
        frames += (b.samples().len() / spec.channels.count().max(1)) as u64;
        printer.consume(b.samples());
    }
    printer.finish();
    let items = printer.fingerprint().to_vec();
    if items.is_empty() {
        return Err("too little audio to fingerprint".into());
    }
    Ok(Print {
        items,
        seconds: frames as f64 / f64::from(rate),
    })
}

/// YouTube's Opus (WebM, what lull plays): symphonia demuxes it, ruopus (pure Rust) decodes it.
/// Opus always decodes at 48 kHz.
fn opus(
    mut format: Box<dyn symphonia::core::formats::FormatReader>,
    track: u32,
    channels: u32,
    mut printer: Fingerprinter,
    limit_s: Option<f64>,
) -> Result<Print, String> {
    const RATE: u32 = 48_000;
    let channels = channels.clamp(1, 2);
    printer.start(RATE, channels).map_err(|e| format!("{e:?}"))?;
    let mut decoder = ruopus::OpusDecoder::with_rate(RATE, channels as usize);
    let limit = limit_s.map(|s| (s * f64::from(RATE)) as u64);
    let mut frames = 0u64;
    while limit.is_none_or(|l| frames < l) {
        let Ok(packet) = format.next_packet() else { break };
        if packet.track_id() != track {
            continue;
        }
        if let Ok(pcm) = decoder.decode_packet_i16(&packet.data) {
            frames += (pcm.len() / channels as usize) as u64;
            printer.consume(&pcm);
        }
    }
    printer.finish();
    let items = printer.fingerprint().to_vec();
    if items.is_empty() {
        return Err("too little audio to fingerprint".into());
    }
    Ok(Print {
        items,
        seconds: frames as f64 / f64::from(RATE),
    })
}

/// The offset of `reference` in `copy` with the fewest differing bits, over at least
/// `MIN_OVERLAP_S` (or all of the reference, when it is shorter). `None` when they can't overlap
/// that much.
pub fn align(reference: &[u32], copy: &[u32]) -> Option<Score> {
    let item = item_s();
    let (n, m) = (reference.len() as i64, copy.len() as i64);
    let need = ((MIN_OVERLAP_S / item) as i64).min(n).max(1);
    let mut best: Option<(f64, i64, i64)> = None;
    for off in -(n - need)..=(m - need) {
        let (from, to) = ((-off).max(0), n.min(m - off));
        if to - from < need {
            continue;
        }
        let errs: u32 = (from..to)
            .map(|i| (reference[i as usize] ^ copy[(i + off) as usize]).count_ones())
            .sum();
        let ber = f64::from(errs) / ((to - from) as f64 * 32.0);
        if best.is_none_or(|(b, _, _)| ber < b) {
            best = Some((ber, off, to - from));
        }
    }
    best.map(|(score, off, overlap)| Score {
        score,
        offset_s: off as f64 * item,
        overlap_s: overlap as f64 * item,
    })
}

/// A print in the compact text the verdict cache and fixtures keep: base64 of little-endian items.
pub fn encode(items: &[u32]) -> String {
    let bytes: Vec<u8> = items.iter().flat_map(|i| i.to_le_bytes()).collect();
    base64(&bytes)
}

pub fn decode(text: &str) -> Option<Vec<u32>> {
    let bytes = unbase64(text)?;
    let (items, rest) = bytes.as_chunks::<4>();
    rest.is_empty()
        .then(|| items.iter().map(|c| u32::from_le_bytes(*c)).collect())
}

/// AcoustID's form of a print: Chromaprint's compressed fingerprint, base64 URL-safe unpadded.
pub fn acoustid_form(items: &[u32]) -> String {
    let config = config();
    let compressed = rusty_chromaprint::FingerprintCompressor::from(&config).compress(items);
    base64(&compressed)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_owned()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn unbase64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = B64.iter().position(|b| *b == c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zones_follow_the_measured_scores() {
        assert_eq!(Zone::of(0.04), Zone::Same, "a song's own upload");
        assert_eq!(Zone::of(0.16), Zone::Near, "another master of it");
        assert_eq!(Zone::of(0.28), Zone::Other, "a re-recording");
        assert_eq!(Zone::of(0.45), Zone::Other, "another song");
    }

    #[test]
    fn a_reference_is_found_anywhere_in_a_copy() -> Result<(), String> {
        // A pseudo-random "song", and a 30 s "preview" cut from 40 s into it with a few flipped bits.
        let mut x = 0x9e37_79b9u32;
        let song: Vec<u32> = (0..2000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x
            })
            .collect();
        let at = (40.0 / item_s()) as usize;
        let preview: Vec<u32> = song[at..at + 240].iter().map(|v| v ^ 0b1001).collect();
        let s = align(&preview, &song).ok_or("no overlap")?;
        assert!((s.offset_s - 40.0).abs() < 0.2, "{s:?}");
        assert!(s.score < 0.07, "{s:?}");
        let other: Vec<u32> = song.iter().map(|v| v.rotate_left(7)).collect();
        assert!(align(&preview, &other).ok_or("no overlap")?.score > 0.3);
        Ok(())
    }

    #[test]
    fn prints_survive_the_cache_and_encode_for_acoustid() {
        let items = vec![0, 1, u32::MAX, 0xdead_beef, 42];
        assert_eq!(decode(&encode(&items)), Some(items.clone()));
        let form = acoustid_form(&items);
        assert!(!form.contains(['+', '/', '=']), "{form}");
    }

    /// YouTube's Opus decodes through ruopus: point `LULL_SOUND_OPUS` at a WebM upload and
    /// `LULL_SOUND_PREVIEW` at the song's preview (no audio is kept in the repository).
    #[test]
    #[ignore = "needs a local WebM upload and its preview"]
    fn a_webm_upload_decodes_and_matches_its_preview() -> Result<(), Box<dyn std::error::Error>> {
        let upload = print_file(Path::new(&std::env::var("LULL_SOUND_OPUS")?), None)?;
        let preview = print_file(Path::new(&std::env::var("LULL_SOUND_PREVIEW")?), None)?;
        let s = align(&preview.items, &upload.items).ok_or("no overlap")?;
        assert_eq!(Zone::of(s.score), Zone::Same, "{s:?}");
        Ok(())
    }

    #[test]
    fn a_wav_decodes_and_prints() -> Result<(), Box<dyn std::error::Error>> {
        // Three seconds of a rising tone: enough audio for Chromaprint's first items.
        let dir = std::env::temp_dir().join(format!("lull-sound-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("tone.wav");
        let rate = 11_025u32;
        let samples: Vec<i16> = (0..rate * 3)
            .map(|i| {
                let t = f64::from(i) / f64::from(rate);
                ((t * (440.0 + 200.0 * t) * std::f64::consts::TAU).sin() * 12_000.0) as i16
            })
            .collect();
        let mut wav = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(&path, wav)?;
        let print = print_file(&path, None)?;
        assert!((print.seconds - 3.0).abs() < 0.01, "{}", print.seconds);
        assert!(!print.items.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
