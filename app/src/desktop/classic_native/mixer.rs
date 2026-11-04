//! The sound the output asks for, buffer by buffer, as the web player's output makes it (web/app.js,
//! section 8b): the playing deck at its loudness level; into the next deck without a gap when the
//! song really reached its end (within 2 s of its length; an early end stops and says so); an
//! equal-power crossfade over the song's last seconds; a better copy faded in over 200 ms once it is
//! lined up; then the equaliser, then the volume (the slider squared, as the page sets it). The
//! visualiser sees the sound after the equaliser, before the volume, as Winamp's did. What happened
//! (a handover, an end, a switch) waits for the engine to read.

use super::deck::{Deck, Phase};
use super::eq::Chain;
use std::collections::VecDeque;
use std::sync::Arc;

/// Mono samples kept for the visualiser.
const TAP: usize = 1024;
/// A song that ends within this long of its length ended by itself: the next one follows at once.
pub const GAPLESS_WITHIN: f64 = 2.0;

/// What the mixer did that the engine needs to know.
#[derive(Debug, PartialEq)]
pub enum Event {
    /// The next deck took over (gapless, or a crossfade's end): it is the current one now.
    Handover,
    /// The song ended and nothing took over.
    Ended,
    /// The better copy took over.
    Switched,
}

/// A crossfade under way: its length and how far it got, in frames.
struct Fade {
    frames: u64,
    done: u64,
}

/// A better copy fading in: frames to skip from it first (it sounds that much ahead) or silence to
/// give it (behind), then the fade.
pub struct Switch {
    pub deck: Arc<Deck>,
    pub skip: i64,
    pub level: f32,
    fade: Fade,
}

pub struct Mixer {
    pub current: Option<Arc<Deck>>,
    pub next: Option<Arc<Deck>>,
    /// The loudness level of the current and the next deck (normalisation), as gains.
    pub level: f32,
    pub next_level: f32,
    /// The slider's value; it plays at its square.
    pub volume: f32,
    pub eq: Option<Chain>,
    fade: Option<Fade>,
    switch: Option<Switch>,
    pub tap: VecDeque<f32>,
    /// The current deck's sound so far, for measured loudness: the sum of squares and frames.
    pub heard: (f64, u64),
    pub events: Vec<Event>,
    /// Whether the song ended (and nothing took over): silence until the engine moves on.
    ended: bool,
    /// The playing song's sound from a moment on (its own time), mono, for lining a better copy up.
    recording: Option<(f64, Vec<f32>)>,
    rate: u32,
    scratch: Vec<f32>,
    other: Vec<f32>,
}

impl Mixer {
    pub fn new(rate: u32) -> Self {
        Self {
            current: None,
            next: None,
            level: 1.0,
            next_level: 1.0,
            volume: 1.0,
            eq: None,
            fade: None,
            switch: None,
            tap: VecDeque::with_capacity(TAP),
            heard: (0.0, 0),
            events: Vec::new(),
            ended: false,
            recording: None,
            rate,
            scratch: Vec::new(),
            other: Vec::new(),
        }
    }

    /// A new current deck: what played stops.
    pub fn play(&mut self, deck: Arc<Deck>, level: f32) {
        self.current = Some(deck);
        self.recording = None;
        self.level = level;
        self.fade = None;
        self.switch = None;
        self.ended = false;
        self.heard = (0.0, 0);
    }

    /// Starts a crossfade into the next deck over `seconds`.
    pub fn crossfade(&mut self, seconds: f64) {
        if self.next.is_some() && self.fade.is_none() {
            self.fade = Some(Fade {
                frames: (seconds.max(0.01) * f64::from(self.rate)) as u64,
                done: 0,
            });
        }
    }

    /// Abandons a crossfade (a pause, a seek): the next deck waits for its turn again.
    pub fn abandon_fade(&mut self) -> bool {
        self.fade.take().is_some()
    }

    pub fn fading(&self) -> bool {
        self.fade.is_some()
    }

    /// Fades the better copy in over `seconds`, after giving or taking `skip` frames to line it up.
    pub fn switch(&mut self, deck: Arc<Deck>, skip: i64, level: f32, seconds: f64) {
        self.switch = Some(Switch {
            deck,
            skip,
            level,
            fade: Fade {
                frames: (seconds * f64::from(self.rate)) as u64,
                done: 0,
            },
        });
    }

    /// The output's buffer: interleaved stereo.
    pub fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let mut at = 0;
        while at < frames {
            let Some(current) = self.current.clone() else {
                break;
            };
            if self.ended {
                break;
            }
            self.scratch.resize((frames - at) * 2, 0.0);
            let from = current.position();
            let got = current.take(&mut self.scratch);
            self.measure(got);
            self.keep(from, got);
            for (i, sample) in self.scratch[..got * 2].iter().enumerate() {
                out[at * 2 + i] = sample * self.level;
            }
            self.mix_fade(&mut out[at * 2..(at + got) * 2]);
            self.mix_switch(&mut out[at * 2..(at + got) * 2]);
            at += got;
            if at < frames && !self.move_on(&current) {
                break;
            }
        }
        self.finish(out);
    }

    /// The current deck ran dry: the next takes over if the song really ended, else it ended (or it
    /// is waiting for sound, and silence fills in). Whether to carry on in this buffer.
    fn move_on(&mut self, current: &Deck) -> bool {
        if !current.drained() {
            return false;
        }
        let complete = current
            .duration()
            .is_none_or(|length| current.position() >= length - GAPLESS_WITHIN);
        match self.next.take() {
            Some(next) if complete && next.phase() != Phase::Opening => {
                self.current = Some(next);
                self.level = self.next_level;
                self.fade = None;
                self.heard = (0.0, 0);
                self.events.push(Event::Handover);
                true
            }
            other => {
                self.next = other;
                self.ended = true;
                self.fade = None;
                self.events.push(Event::Ended);
                false
            }
        }
    }

    /// A crossfade: the next deck under the current, equal-power, until the fade is done and the
    /// next deck takes over.
    fn mix_fade(&mut self, out: &mut [f32]) {
        let (Some(fade), Some(next)) = (self.fade.as_mut(), self.next.clone()) else {
            return;
        };
        self.other.resize(out.len(), 0.0);
        let got = next.take(&mut self.other);
        for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = ((fade.done + i as u64) as f64 / fade.frames as f64).min(1.0);
            let (down, up) = (
                (t * std::f64::consts::FRAC_PI_2).cos() as f32,
                (t * std::f64::consts::FRAC_PI_2).sin() as f32,
            );
            let incoming = if i < got {
                [self.other[i * 2], self.other[i * 2 + 1]]
            } else {
                [0.0, 0.0]
            };
            frame[0] = frame[0] * down + incoming[0] * self.next_level * up;
            frame[1] = frame[1] * down + incoming[1] * self.next_level * up;
        }
        fade.done += (out.len() / 2) as u64;
        if fade.done >= fade.frames {
            self.current = self.next.take();
            self.level = self.next_level;
            self.fade = None;
            self.heard = (0.0, 0);
            self.events.push(Event::Handover);
        }
    }

    /// A better copy: lined up, then faded in over the current deck; once in, it is the current one.
    fn mix_switch(&mut self, out: &mut [f32]) {
        let Some(switch) = self.switch.as_mut() else {
            return;
        };
        while switch.skip > 0 {
            let mut drop = vec![0.0; (switch.skip.min(4096) * 2) as usize];
            let got = switch.deck.take(&mut drop);
            if got == 0 {
                return;
            }
            switch.skip -= got as i64;
        }
        let frames = out.len() / 2;
        let silent = (-switch.skip).clamp(0, frames as i64) as usize;
        switch.skip += silent as i64;
        self.other.resize((frames - silent) * 2, 0.0);
        let got = switch.deck.take(&mut self.other);
        for (i, frame) in out.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let t = ((switch.fade.done + i as u64) as f64 / switch.fade.frames.max(1) as f64).min(1.0);
            let (down, up) = (
                (t * std::f64::consts::FRAC_PI_2).cos() as f32,
                (t * std::f64::consts::FRAC_PI_2).sin() as f32,
            );
            let incoming = match i.checked_sub(silent) {
                Some(j) if j < got => [self.other[j * 2], self.other[j * 2 + 1]],
                _ => [0.0, 0.0],
            };
            frame[0] = frame[0] * down + incoming[0] * switch.level * up;
            frame[1] = frame[1] * down + incoming[1] * switch.level * up;
        }
        switch.fade.done += frames as u64;
        if switch.fade.done >= switch.fade.frames {
            let switch = self.switch.take();
            if let Some(switch) = switch {
                self.current = Some(switch.deck);
                self.level = switch.level;
                self.heard = (0.0, 0);
                self.events.push(Event::Switched);
            }
        }
    }

    /// Starts recording the playing song from `anchor` seconds of it.
    pub fn record(&mut self, anchor: f64) {
        self.recording = Some((anchor, Vec::new()));
    }

    /// What was recorded since the last call.
    pub fn recorded(&mut self) -> Vec<f32> {
        self.recording
            .as_mut()
            .map(|(_, r)| std::mem::take(r))
            .unwrap_or_default()
    }

    /// The frames just taken (the first at `from` seconds), into the recording past its anchor.
    fn keep(&mut self, from: f64, frames: usize) {
        let Some((anchor, recording)) = self.recording.as_mut() else {
            return;
        };
        let rate = f64::from(self.rate);
        for (i, frame) in self.scratch[..frames * 2].as_chunks::<2>().0.iter().enumerate() {
            if from + i as f64 / rate >= *anchor {
                recording.push((frame[0] + frame[1]) / 2.0);
            }
        }
    }

    /// The current deck's level, for measured loudness.
    fn measure(&mut self, frames: usize) {
        let sum: f64 = self.scratch[..frames * 2]
            .iter()
            .map(|s| f64::from(*s) * f64::from(*s))
            .sum();
        self.heard.0 += sum / 2.0;
        self.heard.1 += frames as u64;
    }

    /// The equaliser, the visualiser's copy, and the volume.
    fn finish(&mut self, out: &mut [f32]) {
        let volume = self.volume * self.volume;
        for frame in out.as_chunks_mut::<2>().0 {
            if let Some(chain) = self.eq.as_mut() {
                frame[0] = chain.process(0, frame[0]);
                frame[1] = chain.process(1, frame[1]);
            }
            if self.tap.len() == TAP {
                self.tap.pop_front();
            }
            self.tap.push_back((frame[0] + frame[1]) / 2.0);
            frame[0] *= volume;
            frame[1] *= volume;
        }
    }
}
