//! fptrial: fingerprint a reference clip and candidate files, report the best alignment.
//! Usage: fptrial <reference> <candidate>...   → one JSON line per candidate.
use std::time::Instant;

use rusty_chromaprint::{match_fingerprints, Configuration, Fingerprinter};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

struct Print {
    fp: Vec<u32>,
    seconds: f64,
    decode_ms: f64,
}

fn print(path: &str, config: &Configuration) -> Result<Print, String> {
    let t = Instant::now();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("no track")?.clone();
    let rate = track.codec_params.sample_rate.unwrap_or(48_000);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u32;
    let mut fpr = Fingerprinter::new(config);
    let mut frames = 0u64;
    if track.codec_params.codec == symphonia::core::codecs::CODEC_TYPE_OPUS {
        // symphonia demuxes WebM but has no Opus decoder; ruopus (pure Rust) decodes the packets.
        fpr.start(48_000, channels).map_err(|e| format!("{e:?}"))?;
        let mut opus = ruopus::OpusDecoder::with_rate(48_000, channels as usize);
        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track.id {
                continue;
            }
            if let Ok(pcm) = opus.decode_packet_i16(&packet.data) {
                frames += (pcm.len() / channels as usize) as u64;
                fpr.consume(&pcm);
            }
        }
        fpr.finish();
        return Ok(Print {
            fp: fpr.fingerprint().to_vec(),
            seconds: frames as f64 / 48_000.0,
            decode_ms: t.elapsed().as_secs_f64() * 1000.0,
        });
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("codec: {e}"))?;
    fpr.start(rate, channels).map_err(|e| format!("{e:?}"))?;
    let mut buf: Option<SampleBuffer<i16>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(_) => break,
        };
        let spec = *decoded.spec();
        if buf.as_ref().map_or(true, |b| b.capacity() < decoded.capacity() * spec.channels.count()) {
            buf = Some(SampleBuffer::new(decoded.capacity() as u64, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        frames += (b.samples().len() / spec.channels.count()) as u64;
        fpr.consume(b.samples());
    }
    fpr.finish();
    Ok(Print {
        fp: fpr.fingerprint().to_vec(),
        seconds: frames as f64 / rate as f64,
        decode_ms: t.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Best offset of `short` inside `long` by bit error rate over the overlap (brute force).
fn best(short: &[u32], long: &[u32]) -> (f64, i64, usize) {
    let mut best = (1.0f64, 0i64, 0usize);
    let n = short.len() as i64;
    let m = long.len() as i64;
    let min_overlap = (n / 2).max(20);
    for off in -(n - min_overlap)..=(m - min_overlap) {
        let (mut errs, mut cnt) = (0u32, 0usize);
        for i in 0..n {
            let j = i + off;
            if j < 0 || j >= m {
                continue;
            }
            errs += (short[i as usize] ^ long[j as usize]).count_ones();
            cnt += 1;
        }
        if cnt as i64 >= min_overlap {
            let ber = errs as f64 / (cnt as f64 * 32.0);
            if ber < best.0 {
                best = (ber, off, cnt);
            }
        }
    }
    best
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = Configuration::preset_test2();
    let item = config.item_duration_in_seconds() as f64;
    let reference = match print(&args[0], &config) {
        Ok(p) => p,
        Err(e) => {
            println!("{}", serde_json::json!({"reference": args[0], "error": e}));
            return;
        }
    };
    for cand in &args[1..] {
        let out = match print(cand, &config) {
            Err(e) => serde_json::json!({"candidate": cand, "error": e}),
            Ok(c) => {
                let t = Instant::now();
                let (ber, off, cnt) = best(&reference.fp, &c.fp);
                let segs = match_fingerprints(&reference.fp, &c.fp, &config).unwrap_or_default();
                let covered: f64 = segs.iter().filter(|s| s.score < 10.0).map(|s| s.items_count as f64 * item).sum();
                let seg_best = segs.iter().map(|s| s.score).fold(32.0, f64::min);
                serde_json::json!({
                    "candidate": cand,
                    "ber": (ber * 1000.0).round() / 1000.0,
                    "offset_s": (off as f64 * item * 10.0).round() / 10.0,
                    "overlap_s": (cnt as f64 * item * 10.0).round() / 10.0,
                    "seg_covered_s": (covered * 10.0).round() / 10.0,
                    "seg_best": (seg_best * 100.0).round() / 100.0,
                    "ref_s": (reference.seconds * 10.0).round() / 10.0,
                    "cand_s": (c.seconds * 10.0).round() / 10.0,
                    "ref_ms": reference.decode_ms.round(),
                    "cand_ms": c.decode_ms.round(),
                    "match_ms": (t.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0,
                })
            }
        };
        println!("{out}");
    }
}
