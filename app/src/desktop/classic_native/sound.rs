//! Classic mode's own player, so no webview has to stay open for the sound: songs stream from the
//! hub (ranged requests, so a seek fetches from there on), Symphonia decodes them (Opus, YouTube's
//! format, through opus-decoder: Symphonia has no Opus), the device's equaliser filters them
//! (eq.rs) and cpal plays them at the file's own rate (the system resamples: WASAPI's
//! AUTOCONVERTPCM).
//!
//! One thread per player decodes ahead into a half-second buffer the device drains. Everything the
//! two share sits behind one lock, and one condition wakes the thread: a command, room in the
//! buffer, or the buffer run dry at a song's end. Which song comes next is the player's own call
//! (Next, a song's end, shuffle and repeat), so a song that ended by itself is told apart from one
//! the listener moved on from (`Now::ends`).

use super::eq::Chain;
use crate::desktop::classic::Equaliser;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, CODEC_TYPE_OPUS, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::errors::Error as DecodeError;
use symphonia::core::formats::{FormatOptions, FormatReader, Packet, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;
use ureq::tls::{RootCerts, TlsConfig};

/// Decoded sound kept ahead of the device, in seconds.
const AHEAD: f64 = 0.5;
/// Mono samples kept for the visualiser.
const TAP: usize = 1024;
/// A seek this close ahead reads on through the open response instead of asking again.
const SKIP_READ: u64 = 64 * 1024;
/// Opus always decodes at 48 kHz; a packet holds at most 120 ms of it.
const OPUS_RATE: u32 = 48_000;
const OPUS_MAX_FRAMES: usize = 5_760;

/// A song to play: its stream on the hub, and what the hub knows of its length.
#[derive(Clone)]
pub struct Song {
    pub url: String,
    pub duration: Option<f64>,
}

/// Where the player starts: as `main` had it.
pub struct Start {
    pub index: usize,
    pub position: f64,
    pub playing: bool,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: bool,
    pub eq: Equaliser,
}

/// What the window shows.
#[derive(Clone, Debug, Default)]
pub struct Now {
    pub index: usize,
    pub position: f64,
    pub duration: Option<f64>,
    pub playing: bool,
    /// Fetching or opening a song.
    pub loading: bool,
    /// Stopped (Winamp's Stop): nothing sounds and the time is blank.
    pub stopped: bool,
    pub kbps: Option<u32>,
    pub khz: Option<u32>,
    pub stereo: bool,
    pub error: Option<String>,
    /// How many songs have ended by themselves since the player started.
    pub ends: u32,
}

enum Command {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    Jump(usize),
    Seek(f64),
    Modes { shuffle: bool, repeat: bool },
    Quit,
}

/// The player; dropping it ends its thread (once a fetch in progress returns) and the sound.
pub struct Player {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// The volume as f32 bits: the device reads it on every buffer.
    volume: AtomicU32,
}

#[derive(Default)]
struct State {
    commands: VecDeque<Command>,
    /// Interleaved stereo at `rate`, between the decoder and the device.
    samples: VecDeque<f32>,
    /// The decoder reached the song's end; once `samples` is empty too, the song is over.
    input_done: bool,
    drained: bool,
    rate: u32,
    /// Frames the device has played since `base` seconds into the song.
    played: u64,
    base: f64,
    tap: VecDeque<f32>,
    /// The equaliser's settings, and its filters at `rate` (none when they'd change nothing).
    eq: Option<Equaliser>,
    chain: Option<Chain>,
    ends: u32,
    /// The list, which the window may shorten while a song plays.
    songs: Vec<Song>,
    now: Now,
}

impl State {
    fn set_rate(&mut self, rate: u32) {
        self.rate = rate;
        self.chain = self.eq.as_ref().and_then(|eq| Chain::new(eq, rate));
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Player {
    pub fn start(token: String, songs: Vec<Song>, start: Start) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                now: Now {
                    index: start.index,
                    position: start.position,
                    ..Now::default()
                },
                eq: Some(start.eq.clone()),
                songs,
                ..State::default()
            }),
            wake: Condvar::new(),
            volume: AtomicU32::new((start.volume.clamp(0.0, 1.0) as f32).to_bits()),
        });
        let worker = Worker::new(shared.clone(), token, start.shuffle, start.repeat);
        std::thread::Builder::new()
            .name("classic-player".into())
            .spawn(move || worker.run(start.index, start.position, start.playing))
            .map_err(|e| e.to_string())?;
        Ok(Self { shared })
    }

    pub fn play(&self) {
        self.send(Command::Play);
    }
    pub fn pause(&self) {
        self.send(Command::Pause);
    }
    pub fn stop(&self) {
        self.send(Command::Stop);
    }
    pub fn next(&self) {
        self.send(Command::Next);
    }
    pub fn previous(&self) {
        self.send(Command::Previous);
    }
    pub fn jump(&self, index: usize) {
        self.send(Command::Jump(index));
    }
    /// Seek `seconds` into the song.
    pub fn seek(&self, seconds: f64) {
        self.send(Command::Seek(seconds));
    }
    pub fn set_modes(&self, shuffle: bool, repeat: bool) {
        self.send(Command::Modes { shuffle, repeat });
    }
    /// Takes songs out of the list: `keep` says, for each one, whether it stays. The playing song
    /// always does. The list and the playing song's place change together, at once.
    pub fn keep(&self, keep: &[bool]) {
        let mut state = lock(&self.shared);
        let index = state.now.index;
        let stays = |i: usize| i == index || keep.get(i).copied().unwrap_or(true);
        state.now.index = (0..index).filter(|&i| stays(i)).count();
        let mut i = 0;
        state.songs.retain(|_| {
            i += 1;
            stays(i - 1)
        });
    }

    /// The equaliser changes at the next buffer the device asks for.
    pub fn set_equaliser(&self, eq: Equaliser) {
        let mut state = lock(&self.shared);
        let rate = state.rate;
        state.eq = Some(eq);
        state.set_rate(rate);
    }

    pub fn volume(&self) -> f64 {
        f64::from(f32::from_bits(self.shared.volume.load(Ordering::Relaxed)))
    }
    pub fn set_volume(&self, volume: f64) {
        let volume = volume.clamp(0.0, 1.0) as f32;
        self.shared.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn now(&self) -> Now {
        let state = lock(&self.shared);
        let mut now = state.now.clone();
        now.ends = state.ends;
        if state.rate > 0 && !now.stopped {
            now.position = state.base + state.played as f64 / f64::from(state.rate);
        }
        if let Some(length) = now.duration {
            now.position = now.position.min(length);
        }
        now
    }

    /// The newest `count` mono samples the device played, for the visualiser.
    pub fn tap(&self, count: usize) -> Vec<f32> {
        let state = lock(&self.shared);
        let skip = state.tap.len().saturating_sub(count);
        state.tap.iter().skip(skip).copied().collect()
    }

    fn send(&self, command: Command) {
        lock(&self.shared).commands.push_back(command);
        self.shared.wake.notify_all();
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.send(Command::Quit);
    }
}

/// The open song: its demuxer and decoder, and the device stream draining the buffer.
struct Loaded {
    format: Box<dyn FormatReader>,
    decoder: Sound,
    track: u32,
    stream: cpal::Stream,
}

/// A song's decoder: Symphonia's, or opus-decoder for Opus.
enum Sound {
    Symphonia(Box<dyn Decoder>),
    Opus {
        decoder: Box<opus_decoder::OpusDecoder>,
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
        let decoder = opus_decoder::OpusDecoder::new(OPUS_RATE, channels).map_err(|e| e.to_string())?;
        Ok(Sound::Opus {
            decoder: Box::new(decoder),
            channels,
            pcm: vec![0.0; OPUS_MAX_FRAMES * channels],
        })
    }

    fn reset(&mut self) {
        match self {
            Sound::Symphonia(decoder) => decoder.reset(),
            Sound::Opus { decoder, channels, .. } => {
                if let Ok(fresh) = opus_decoder::OpusDecoder::new(OPUS_RATE, *channels) {
                    **decoder = fresh;
                }
            }
        }
    }

    /// One packet's sound as stereo frames, onto the end of `out`. A damaged packet adds nothing.
    fn decode(&mut self, packet: &Packet, out: &mut VecDeque<f32>) -> Result<(), String> {
        let mut push = |samples: &[f32], channels: usize| {
            for frame in samples.chunks_exact(channels.max(1)) {
                out.push_back(frame[0]);
                out.push_back(frame.get(1).copied().unwrap_or(frame[0]));
            }
        };
        match self {
            Sound::Symphonia(decoder) => {
                let decoded = match decoder.decode(packet) {
                    Ok(decoded) => decoded,
                    // A damaged packet is skipped, as every player does.
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

struct Worker {
    shared: Arc<Shared>,
    agent: ureq::Agent,
    token: String,
    shuffle: bool,
    repeat: bool,
    loaded: Option<Loaded>,
    playing: bool,
    random: RandomState,
    draws: u64,
}

impl Worker {
    fn new(shared: Arc<Shared>, token: String, shuffle: bool, repeat: bool) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(20)))
            .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
            .user_agent(concat!("lull-app/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self {
            shared,
            agent,
            token,
            shuffle,
            repeat,
            loaded: None,
            playing: false,
            random: RandomState::new(),
            draws: 0,
        }
    }

    fn run(mut self, index: usize, position: f64, playing: bool) {
        self.playing = playing;
        self.load(index, position);
        loop {
            let (commands, drained, room) = self.wait();
            for command in commands {
                match command {
                    Command::Quit => return,
                    Command::Play => self.play(),
                    Command::Pause => self.pause(),
                    Command::Stop => self.stop(),
                    Command::Next => self.next(),
                    Command::Previous => self.previous(),
                    Command::Jump(index) => self.jump(index),
                    Command::Seek(seconds) => self.seek(seconds),
                    Command::Modes { shuffle, repeat } => (self.shuffle, self.repeat) = (shuffle, repeat),
                }
            }
            if drained {
                lock(&self.shared).ends += 1;
                self.next();
            } else if room {
                self.decode();
            }
        }
    }

    /// Sleeps until there is something to do: commands to run, the song over, or room to decode.
    fn wait(&self) -> (Vec<Command>, bool, bool) {
        let decoding = self.loaded.is_some();
        let mut state = lock(&self.shared);
        loop {
            let room = decoding && !state.input_done && (state.samples.len() as f64) < capacity(state.rate);
            if !state.commands.is_empty() || state.drained || room {
                let drained = std::mem::take(&mut state.drained);
                return (state.commands.drain(..).collect(), drained, room);
            }
            state = self
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    fn load(&mut self, index: usize, position: f64) {
        self.loaded = None;
        {
            let mut state = lock(&self.shared);
            reset_buffer(&mut state, position);
            state.now = Now {
                index,
                position,
                loading: true,
                ..Now::default()
            };
        }
        let song = lock(&self.shared).songs.get(index).cloned();
        let opened = song
            .as_ref()
            .ok_or_else(|| "no such song".to_owned())
            .and_then(|song| open(&self.agent, &self.token, song, position));
        let mut state = lock(&self.shared);
        state.now.loading = false;
        match opened.and_then(|(opened, facts)| {
            let stream = output(&self.shared, facts.rate)?;
            Ok((opened, facts, stream))
        }) {
            Ok(((format, decoder, track), facts, stream)) => {
                state.set_rate(facts.rate);
                state.now.duration = facts.duration.or(song.and_then(|s| s.duration));
                state.now.khz = Some((facts.rate + 500) / 1000);
                state.now.kbps = facts.kbps;
                state.now.stereo = facts.channels > 1;
                state.now.playing = self.playing;
                if self.playing
                    && let Err(e) = stream.play()
                {
                    state.now.error = Some(e.to_string());
                }
                self.loaded = Some(Loaded {
                    format,
                    decoder,
                    track,
                    stream,
                });
            }
            Err(error) => {
                state.now.error = Some(error);
                state.now.stopped = true;
                self.playing = false;
            }
        }
    }

    fn decode(&mut self) {
        let Some(loaded) = self.loaded.as_mut() else {
            return;
        };
        let packet = loop {
            match loaded.format.next_packet() {
                Ok(packet) if packet.track_id() == loaded.track => break Some(packet),
                Ok(_) => continue,
                Err(DecodeError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break None,
                Err(DecodeError::ResetRequired) => {
                    loaded.decoder.reset();
                    continue;
                }
                Err(e) => {
                    lock(&self.shared).now.error = Some(e.to_string());
                    break None;
                }
            }
        };
        let Some(packet) = packet else {
            lock(&self.shared).input_done = true;
            self.shared.wake.notify_all();
            return;
        };
        let mut sound = VecDeque::new();
        let decoded = loaded.decoder.decode(&packet, &mut sound);
        let mut state = lock(&self.shared);
        match decoded {
            Ok(()) => state.samples.extend(sound),
            Err(error) => {
                state.now.error = Some(error);
                state.input_done = true;
            }
        }
    }

    fn play(&mut self) {
        let stopped = lock(&self.shared).now.stopped;
        if stopped || self.loaded.is_none() {
            let index = lock(&self.shared).now.index;
            self.playing = true;
            return self.load(index, 0.0);
        }
        self.playing = true;
        self.set_stream(true);
    }

    fn pause(&mut self) {
        self.playing = false;
        self.set_stream(false);
    }

    fn set_stream(&mut self, playing: bool) {
        let Some(loaded) = &self.loaded else {
            return;
        };
        let result = if playing {
            loaded.stream.play()
        } else {
            loaded.stream.pause()
        };
        let mut state = lock(&self.shared);
        state.now.playing = playing;
        if let Err(e) = result {
            state.now.error = Some(e.to_string());
        }
    }

    fn stop(&mut self) {
        self.playing = false;
        self.loaded = None;
        let mut state = lock(&self.shared);
        reset_buffer(&mut state, 0.0);
        state.now.playing = false;
        state.now.stopped = true;
    }

    fn jump(&mut self, index: usize) {
        if index < self.count() {
            self.load(index, 0.0);
        }
    }

    /// The next song: another at random with shuffle on; after the last, the first with repeat on,
    /// else the player stops on the last.
    fn next(&mut self) {
        let index = lock(&self.shared).now.index;
        let count = self.count();
        if self.shuffle && count > 1 {
            let pick = self.draw(count - 1);
            return self.load(if pick >= index { pick + 1 } else { pick }, 0.0);
        }
        match (index + 1 < count, self.repeat) {
            (true, _) => self.load(index + 1, 0.0),
            (false, true) => self.load(0, 0.0),
            (false, false) => self.stop(),
        }
    }

    /// Winamp's Previous: the song before, the last one from the first with repeat on.
    fn previous(&mut self) {
        let index = lock(&self.shared).now.index;
        match (index.checked_sub(1), self.repeat) {
            (Some(before), _) => self.load(before, 0.0),
            (None, true) => self.load(self.count().saturating_sub(1), 0.0),
            (None, false) => self.load(0, 0.0),
        }
    }

    fn count(&self) -> usize {
        lock(&self.shared).songs.len()
    }

    /// A number below `below`, at random.
    fn draw(&mut self, below: usize) -> usize {
        self.draws += 1;
        (self.random.hash_one(self.draws) % below as u64) as usize
    }

    fn seek(&mut self, seconds: f64) {
        let Some(loaded) = self.loaded.as_mut() else {
            return;
        };
        let to = SeekTo::Time {
            time: Time::from(seconds.max(0.0)),
            track_id: Some(loaded.track),
        };
        match loaded.format.seek(SeekMode::Accurate, to) {
            Ok(_) => {
                loaded.decoder.reset();
                reset_buffer(&mut lock(&self.shared), seconds.max(0.0));
            }
            Err(e) => lock(&self.shared).now.error = Some(format!("can't seek: {e}")),
        }
    }
}

fn capacity(rate: u32) -> f64 {
    f64::from(rate.max(8_000)) * 2.0 * AHEAD
}

fn reset_buffer(state: &mut State, base: f64) {
    state.samples.clear();
    state.input_done = false;
    state.drained = false;
    state.played = 0;
    state.base = base;
}

/// What the window says of a song once it's open.
struct Facts {
    rate: u32,
    channels: usize,
    duration: Option<f64>,
    kbps: Option<u32>,
}

type Opened = (Box<dyn FormatReader>, Sound, u32);

/// Fetches the song, finds its audio and gets a decoder for it, `position` seconds in.
fn open(agent: &ureq::Agent, token: &str, song: &Song, position: f64) -> Result<(Opened, Facts), String> {
    let source = Remote::open(agent.clone(), &song.url, token)?;
    let bytes = source.length;
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
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
        .map_err(|e| format!("can't read this song: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("the song has no sound")?;
    let params = track.codec_params.clone();
    let id = track.id;
    let decoder = Sound::new(&params)?;
    let rate = match decoder {
        Sound::Opus { .. } => OPUS_RATE,
        Sound::Symphonia(_) => params.sample_rate.ok_or("the song doesn't say its sample rate")?,
    };
    let duration = params.time_base.zip(params.n_frames).map(|(base, frames)| {
        let t = base.calc_time(frames);
        t.seconds as f64 + t.frac
    });
    let kbps = bytes
        .zip(duration)
        .filter(|(_, d)| *d > 0.0)
        .map(|(b, d)| (b as f64 * 8.0 / d / 1000.0).round() as u32);
    if position > 0.0 {
        let to = SeekTo::Time {
            time: Time::from(position),
            track_id: Some(id),
        };
        format
            .seek(SeekMode::Accurate, to)
            .map_err(|e| format!("can't seek: {e}"))?;
    }
    let facts = Facts {
        rate,
        channels: params.channels.map_or(2, |c| c.count()),
        duration,
        kbps,
    };
    Ok(((format, decoder, id), facts))
}

/// The device's stream at the song's rate, in stereo, draining the shared buffer. Created paused.
fn output(shared: &Arc<Shared>, rate: u32) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no sound output on this computer")?;
    let config = cpal::StreamConfig {
        channels: 2,
        sample_rate: rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let shared = shared.clone();
    let stream = device
        .build_output_stream::<f32, _, _>(
            config,
            move |out, _| fill(&shared, out),
            |e| eprintln!("lull: Classic sound output: {e}"),
            None,
        )
        .map_err(|e| format!("can't open the sound output: {e}"))?;
    stream.pause().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// The device wants `out` (interleaved stereo): the buffer's next frames through the equaliser at
/// the volume, silence where it has none. The visualiser sees the sound after the equaliser, before
/// the volume, as Winamp's did.
fn fill(shared: &Shared, out: &mut [f32]) {
    let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    let mut state = lock(shared);
    let state = &mut *state;
    let mut played = 0;
    for frame in out.as_chunks_mut::<2>().0 {
        let (Some(mut left), Some(mut right)) = (state.samples.pop_front(), state.samples.pop_front()) else {
            frame.fill(0.0);
            continue;
        };
        if let Some(chain) = state.chain.as_mut() {
            left = chain.process(0, left);
            right = chain.process(1, right);
        }
        frame[0] = left * volume;
        frame[1] = right * volume;
        if state.tap.len() == TAP {
            state.tap.pop_front();
        }
        state.tap.push_back((left + right) / 2.0);
        played += 1;
    }
    state.played += played;
    if state.samples.is_empty() && state.input_done {
        state.drained = true;
    }
    shared.wake.notify_all();
}

/// A song on the hub as a seekable file: read straight through one response, and a seek past what
/// is close ahead asks for the rest from there (a `Range` request).
struct Remote {
    agent: ureq::Agent,
    url: String,
    token: String,
    length: Option<u64>,
    ranges: bool,
    position: u64,
    body: Option<ureq::BodyReader<'static>>,
}

impl Remote {
    fn open(agent: ureq::Agent, url: &str, token: &str) -> Result<Self, String> {
        let mut remote = Self {
            agent,
            url: url.to_owned(),
            token: token.to_owned(),
            length: None,
            ranges: false,
            position: 0,
            body: None,
        };
        remote.request().map_err(|e| format!("can't fetch the song: {e}"))?;
        Ok(remote)
    }

    /// Asks for the song from `position` on.
    fn request(&mut self) -> io::Result<()> {
        let mut request = self
            .agent
            .get(&self.url)
            .header("Authorization", format!("Bearer {}", self.token));
        if self.position > 0 {
            request = request.header("Range", format!("bytes={}-", self.position));
        }
        let response = request.call().map_err(io::Error::other)?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let partial = response.status() == 206;
        if self.position > 0 && !partial {
            return Err(io::Error::other("the hub can't seek in this song"));
        }
        if self.length.is_none() {
            self.length = if partial {
                header("content-range").and_then(|r| r.rsplit('/').next()?.parse().ok())
            } else {
                header("content-length").and_then(|l| l.parse().ok())
            };
            self.ranges = header("accept-ranges").is_some_and(|r| r.contains("bytes"));
        }
        self.body = Some(response.into_body().into_reader());
        Ok(())
    }
}

impl Read for Remote {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.body.is_none() {
            self.request()?;
        }
        let read = match self.body.as_mut() {
            Some(body) => body.read(buf)?,
            None => 0,
        };
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
        self.ranges && self.length.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.length
    }
}
