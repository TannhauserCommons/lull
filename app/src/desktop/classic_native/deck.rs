//! One song being played: its own thread fetches it from the hub (remote.rs), Symphonia decodes it
//! (Opus, YouTube's format, through libopus: Symphonia has none), and it is resampled to the
//! output's rate and kept a little ahead for the mixer, which takes it frame by frame. A deck plays
//! the current song; another may hold the next (for gapless and crossfade) or a better copy of the
//! current one (to switch to without a jump), as the web player's two decks do.
//!
//! Its position is the song's own: where it started (a load at a second, a seek) plus the frames
//! the mixer has taken.

use super::remote::{self, NOT_ARRIVED, Refused, Remote};
use audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, CODEC_TYPE_OPUS, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Opus always decodes at 48 kHz; a packet holds at most 120 ms of it.
const OPUS_RATE: u32 = 48_000;
const OPUS_MAX_FRAMES: usize = 5_760;
/// The resampler's input, in frames a call.
const CHUNK: usize = 1024;

/// What to play.
#[derive(Clone, Debug)]
pub struct Song {
    pub url: String,
    /// What the hub knows of its length, for while the file doesn't say.
    pub duration: Option<f64>,
}

/// Where a deck stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Fetching and opening.
    Opening,
    Playing,
    /// All of it decoded: once the mixer has taken the rest, the song is over.
    Done,
    /// The response broke off before the song's end (the hub moved to another copy, or the
    /// transfer ended): the web player waits for the next attempt.
    Cut(String),
    /// A seek past what has arrived (the hub's 416): the song must be opened again where it was.
    Denied,
    Failed(Refused),
}

/// What the main window says of a song once it's open.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Facts {
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    pub stereo: bool,
}

pub struct Deck {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    rate: u32,
}

struct State {
    /// Stereo frames at the output's rate, decoded ahead of the mixer.
    samples: VecDeque<f32>,
    /// The position is `base` plus the frames the mixer took since.
    base: f64,
    taken: u64,
    phase: Phase,
    duration: Option<f64>,
    facts: Facts,
    /// How much to keep decoded ahead, in seconds: a deck waiting its turn needs less.
    ahead: f64,
    seek: Option<f64>,
    quit: bool,
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Deck {
    /// Starts fetching `song` from `at` seconds in, decoded at the output's `rate`.
    pub fn open(song: Song, token: String, at: f64, rate: u32, ahead: f64) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                samples: VecDeque::new(),
                base: at,
                taken: 0,
                phase: Phase::Opening,
                duration: song.duration,
                facts: Facts::default(),
                ahead,
                seek: None,
                quit: false,
            }),
            wake: Condvar::new(),
            rate,
        });
        let worker = shared.clone();
        let spawned = std::thread::Builder::new()
            .name("classic-deck".into())
            .spawn(move || decode(&worker, &song, &token, at));
        if let Err(e) = spawned {
            lock(&shared).phase = Phase::Failed(Refused::Offline(e.to_string()));
        }
        Self { shared }
    }

    /// Up to `out.len() / 2` frames, taken; the count. Silence isn't added: the mixer decides.
    pub fn take(&self, out: &mut [f32]) -> usize {
        let mut state = lock(&self.shared);
        let frames = (state.samples.len() / 2).min(out.len() / 2);
        for (slot, sample) in out.iter_mut().zip(state.samples.drain(..frames * 2)) {
            *slot = sample;
        }
        state.taken += frames as u64;
        drop(state);
        self.shared.wake.notify_all();
        frames
    }

    /// Seconds into the song.
    pub fn position(&self) -> f64 {
        let state = lock(&self.shared);
        state.base + state.taken as f64 / f64::from(self.shared.rate)
    }

    pub fn duration(&self) -> Option<f64> {
        lock(&self.shared).duration
    }

    pub fn phase(&self) -> Phase {
        lock(&self.shared).phase.clone()
    }

    pub fn facts(&self) -> Facts {
        lock(&self.shared).facts
    }

    /// The first `frames` decoded and not yet taken, mono, left where they are (a better copy, lined
    /// up before it plays).
    pub fn peek(&self, frames: usize) -> Vec<f32> {
        let state = lock(&self.shared);
        state
            .samples
            .iter()
            .take(frames * 2)
            .collect::<Vec<_>>()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|f| (f[0] + f[1]) / 2.0)
            .collect()
    }

    /// Seconds decoded ahead of the mixer.
    pub fn buffered(&self) -> f64 {
        lock(&self.shared).samples.len() as f64 / 2.0 / f64::from(self.shared.rate)
    }

    /// Everything decoded and taken: the song played to its end.
    pub fn drained(&self) -> bool {
        let state = lock(&self.shared);
        state.phase == Phase::Done && state.samples.is_empty()
    }

    pub fn seek(&self, seconds: f64) {
        let mut state = lock(&self.shared);
        state.seek = Some(seconds.max(0.0));
        drop(state);
        self.shared.wake.notify_all();
    }

    pub fn set_ahead(&self, seconds: f64) {
        lock(&self.shared).ahead = seconds;
        self.shared.wake.notify_all();
    }
}

impl Drop for Deck {
    fn drop(&mut self) {
        lock(&self.shared).quit = true;
        self.shared.wake.notify_all();
    }
}

/// The deck's thread: open, then decode ahead of the mixer until the song is done, a seek moves it,
/// or the deck goes.
fn decode(shared: &Shared, song: &Song, token: &str, at: f64) {
    let mut opened = match open(song, token, at) {
        Ok(opened) => opened,
        Err(refused) => {
            lock(shared).phase = Phase::Failed(refused);
            return;
        }
    };
    let mut resample = match Resample::new(opened.rate, shared.rate) {
        Ok(resample) => resample,
        Err(e) => {
            lock(shared).phase = Phase::Failed(Refused::Status(0, e));
            return;
        }
    };
    {
        let mut state = lock(shared);
        if let Some(duration) = opened.duration {
            state.duration = Some(duration);
        }
        state.facts = opened.facts;
        state.phase = Phase::Playing;
    }
    let mut sound = Vec::new();
    loop {
        let seek = {
            let mut state = lock(shared);
            loop {
                if state.quit {
                    return;
                }
                if state.seek.is_some() {
                    break state.seek.take();
                }
                let room = (state.samples.len() as f64) < state.ahead * f64::from(shared.rate) * 2.0;
                if room && state.phase == Phase::Playing {
                    break None;
                }
                state = shared
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        if let Some(seconds) = seek {
            let to = SeekTo::Time {
                time: Time::from(seconds),
                track_id: Some(opened.track),
            };
            let mut state = lock(shared);
            match opened.format.seek(SeekMode::Accurate, to) {
                Ok(_) => {
                    opened.decoder.reset();
                    resample.reset();
                    state.samples.clear();
                    state.base = seconds;
                    state.taken = 0;
                    state.phase = Phase::Playing;
                }
                Err(DecodeError::IoError(e)) if e.to_string().contains(NOT_ARRIVED) => state.phase = Phase::Denied,
                Err(e) => state.phase = Phase::Cut(format!("can't seek: {e}")),
            }
            continue;
        }
        sound.clear();
        let end = match next_packet(&mut opened) {
            Ok(Some(packet)) => match opened.decoder.decode(&packet, &mut sound) {
                Ok(()) => None,
                Err(e) => Some(Phase::Failed(Refused::Status(0, e))),
            },
            Ok(None) => Some(Phase::Done),
            Err(e) => Some(Phase::Cut(e)),
        };
        let mut out = resample.push(&sound, end.is_some());
        let mut state = lock(shared);
        state.samples.extend(out.drain(..));
        if let Some(end) = end {
            state.phase = end;
        }
    }
}

/// The next packet of the song's track; `None` at its end.
fn next_packet(opened: &mut Opened) -> Result<Option<Packet>, String> {
    loop {
        match opened.format.next_packet() {
            Ok(packet) if packet.track_id() == opened.track => return Ok(Some(packet)),
            Ok(_) => continue,
            Err(DecodeError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof && !cut(&e) => {
                return Ok(None);
            }
            Err(DecodeError::ResetRequired) => opened.decoder.reset(),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// Symphonia reports a file's end as an unexpected end of file; the hub's response breaking off
/// early is one too, told apart by its words (remote.rs).
fn cut(e: &io::Error) -> bool {
    e.to_string().contains("ended the song's response early")
}

/// The song open: its demuxer, decoder and track, and what it says of itself.
struct Opened {
    format: Box<dyn FormatReader>,
    decoder: Sound,
    track: u32,
    rate: u32,
    duration: Option<f64>,
    facts: Facts,
}

/// Fetches the song, finds its audio and a decoder for it, `at` seconds in.
fn open(song: &Song, token: &str, at: f64) -> Result<Opened, Refused> {
    let source = Remote::open(remote::agent(), &song.url, token, 0)?;
    let bytes = source.length();
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let unreadable = |e: DecodeError| Refused::Status(0, format!("can't read this song: {e}"));
    let probed = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions {
                enable_gapless: true,
                ..FormatOptions::default()
            },
            &MetadataOptions::default(),
        )
        .map_err(unreadable)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| Refused::Status(0, "the song has no sound".into()))?;
    let params = track.codec_params.clone();
    let id = track.id;
    let decoder = Sound::new(&params).map_err(|e| Refused::Status(0, e))?;
    let rate = match decoder {
        Sound::Opus { .. } => OPUS_RATE,
        Sound::Symphonia(_) => params
            .sample_rate
            .ok_or_else(|| Refused::Status(0, "the song doesn't say its sample rate".into()))?,
    };
    let duration = params.time_base.zip(params.n_frames).map(|(base, frames)| {
        let t = base.calc_time(frames);
        t.seconds as f64 + t.frac
    });
    let kbps = bytes
        .zip(duration)
        .filter(|(_, d)| *d > 0.0)
        .map(|(b, d)| (b as f64 * 8.0 / d / 1000.0).round() as u32);
    if at > 0.0 {
        let to = SeekTo::Time {
            time: Time::from(at),
            track_id: Some(id),
        };
        format.seek(SeekMode::Accurate, to).map_err(|e| match e {
            DecodeError::IoError(e) if e.to_string().contains(NOT_ARRIVED) => Refused::Status(416, NOT_ARRIVED.into()),
            other => Refused::Status(0, format!("can't seek: {other}")),
        })?;
    }
    let facts = Facts {
        kbps,
        khz: Some((rate + 500) / 1000),
        stereo: params.channels.map_or(2, |c| c.count()) > 1,
    };
    Ok(Opened {
        format,
        decoder,
        track: id,
        rate,
        duration,
        facts,
    })
}

/// A song's decoder: Symphonia's, or libopus for Opus.
enum Sound {
    Symphonia(Box<dyn Decoder>),
    Opus {
        decoder: opus::Decoder,
        channels: usize,
        pcm: Vec<f32>,
    },
}

impl Sound {
    fn new(params: &CodecParameters) -> Result<Self, String> {
        if params.codec != CODEC_TYPE_OPUS {
            return symphonia::default::get_codecs()
                .make(params, &DecoderOptions::default())
                .map(Sound::Symphonia)
                .map_err(|e| format!("can't play this kind of song: {e}"));
        }
        let channels = params.channels.map_or(2, |c| c.count()).clamp(1, 2);
        let layout = if channels == 1 {
            opus::Channels::Mono
        } else {
            opus::Channels::Stereo
        };
        let decoder = opus::Decoder::new(OPUS_RATE, layout).map_err(|e| e.to_string())?;
        Ok(Sound::Opus {
            decoder,
            channels,
            pcm: vec![0.0; OPUS_MAX_FRAMES * channels],
        })
    }

    fn reset(&mut self) {
        match self {
            Sound::Symphonia(decoder) => decoder.reset(),
            Sound::Opus { decoder, .. } => {
                if let Err(e) = decoder.reset_state() {
                    eprintln!("lull: Opus decoder: {e}");
                }
            }
        }
    }

    /// One packet's sound as stereo frames onto `out`. A damaged packet adds nothing, as every
    /// player skips it.
    fn decode(&mut self, packet: &Packet, out: &mut Vec<f32>) -> Result<(), String> {
        let mut push = |samples: &[f32], channels: usize| {
            for frame in samples.chunks_exact(channels.max(1)) {
                out.push(frame[0]);
                out.push(frame.get(1).copied().unwrap_or(frame[0]));
            }
        };
        match self {
            Sound::Symphonia(decoder) => {
                let decoded = match decoder.decode(packet) {
                    Ok(decoded) => decoded,
                    Err(DecodeError::DecodeError(_)) => return Ok(()),
                    Err(e) => return Err(e.to_string()),
                };
                let spec = *decoded.spec();
                let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
                buffer.copy_interleaved_ref(decoded);
                push(buffer.samples(), spec.channels.count());
            }
            Sound::Opus { decoder, channels, pcm } => {
                if let Ok(frames) = decoder.decode_float(&packet.data, pcm, false) {
                    push(&pcm[..frames * *channels], *channels);
                }
            }
        }
        Ok(())
    }
}

/// From the song's rate to the output's (rubato's FFT resampler: cheap, and clean enough to be
/// heard as the browser's), or nothing to do when they match. Its delay is trimmed from the start
/// and its tail flushed at the song's end, so a song is exactly as long after as before.
struct Resample {
    fft: Option<Fft<f32>>,
    ratio: f64,
    pending: Vec<f32>,
    out: Vec<f32>,
    /// Frames in since the start (or the last seek), and frames out after the delay.
    taken_in: u64,
    given_out: u64,
    skip: usize,
}

impl Resample {
    fn new(from: u32, to: u32) -> Result<Self, String> {
        let fft = (from != to)
            .then(|| Fft::<f32>::new(from as usize, to as usize, CHUNK, 2, FixedSync::Input))
            .transpose()
            .map_err(|e| format!("can't resample {from} Hz to {to} Hz: {e}"))?;
        let skip = fft.as_ref().map_or(0, |f| f.output_delay());
        Ok(Self {
            fft,
            ratio: f64::from(to) / f64::from(from),
            pending: Vec::new(),
            out: Vec::new(),
            taken_in: 0,
            given_out: 0,
            skip,
        })
    }

    fn reset(&mut self) {
        if let Some(fft) = self.fft.as_mut() {
            fft.reset();
            self.skip = fft.output_delay();
        }
        self.pending.clear();
        self.taken_in = 0;
        self.given_out = 0;
    }

    /// Stereo frames in; what is ready out. At the end, the rest, its tail included.
    fn push(&mut self, input: &[f32], end: bool) -> Vec<f32> {
        let Some(fft) = self.fft.as_mut() else {
            return input.to_vec();
        };
        self.pending.extend_from_slice(input);
        self.taken_in += (input.len() / 2) as u64;
        let mut ready = Vec::new();
        loop {
            let need = fft.input_frames_next();
            let flushing = end && self.pending.len() / 2 < need;
            if self.pending.len() / 2 < need && !flushing {
                break;
            }
            if flushing {
                let wanted = (self.taken_in as f64 * self.ratio).round() as u64;
                if self.given_out >= wanted {
                    break;
                }
                self.pending.resize(need * 2, 0.0);
            }
            self.out.resize(fft.output_frames_max() * 2, 0.0);
            let processed = InterleavedSlice::new(&self.pending[..need * 2], 2, need)
                .ok()
                .and_then(|input| {
                    let frames = self.out.len() / 2;
                    let mut output = InterleavedSlice::new_mut(&mut self.out, 2, frames).ok()?;
                    fft.process_into_buffer(&input, &mut output, None).ok()
                });
            let Some((used, made)) = processed else {
                break;
            };
            self.pending.drain(..used * 2);
            let mut made = &self.out[..made * 2];
            let skip = self.skip.min(made.len() / 2);
            self.skip -= skip;
            made = &made[skip * 2..];
            if end {
                let wanted = (self.taken_in as f64 * self.ratio).round() as u64;
                let room = wanted.saturating_sub(self.given_out) as usize;
                made = &made[..made.len().min(room * 2)];
            }
            self.given_out += (made.len() / 2) as u64;
            ready.extend_from_slice(made);
        }
        ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_resampled_song_is_as_long_as_before() {
        let Ok(mut resample) = Resample::new(44_100, 48_000) else {
            return assert!(Resample::new(44_100, 48_000).is_ok());
        };
        let tone: Vec<f32> = (0..44_100 * 2).map(|i| ((i / 2) as f32 * 0.05).sin() * 0.5).collect();
        let mut out = Vec::new();
        for chunk in tone.chunks(2 * 700) {
            out.extend(resample.push(chunk, false));
        }
        out.extend(resample.push(&[], true));
        assert_eq!(out.len() / 2, 48_000);
        // The tone's level comes through, its start not delayed into silence.
        assert!(out[200..400].iter().any(|s| s.abs() > 0.1));
    }

    #[test]
    fn the_same_rate_passes_through() {
        let Ok(mut resample) = Resample::new(48_000, 48_000) else {
            return assert!(Resample::new(48_000, 48_000).is_ok());
        };
        assert_eq!(resample.push(&[0.1, 0.2], false), vec![0.1, 0.2]);
    }
}
