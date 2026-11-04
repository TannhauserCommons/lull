//! Where Classic's sound goes: one stream for the whole session, opened at the device's own rate in
//! stereo (every song is resampled to it, so songs follow each other without a gap and crossfade
//! whatever their rates), which asks the mixer for each buffer. Tests pull from a stand-in instead.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Fills an interleaved stereo buffer: the mixer.
pub type Render = Box<dyn FnMut(&mut [f32]) + Send>;

pub trait Output {
    /// Frames a second.
    fn rate(&self) -> u32;
    /// Starts or stops asking for sound; stopped, nothing runs.
    fn run(&self, running: bool) -> Result<(), String>;
}

/// The default output device, through cpal.
pub struct Device {
    stream: cpal::Stream,
    rate: u32,
}

impl Device {
    pub fn open(mut render: Render) -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("no sound output on this computer")?;
        let rate = device
            .default_output_config()
            .map_err(|e| format!("the sound output says nothing of itself: {e}"))?
            .sample_rate();
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate: rate,
            buffer_size: cpal::BufferSize::Default,
        };
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |out, _| render(out),
                |e| eprintln!("lull: Classic sound output: {e}"),
                None,
            )
            .map_err(|e| format!("can't open the sound output: {e}"))?;
        stream.pause().map_err(|e| e.to_string())?;
        Ok(Self { stream, rate })
    }
}

impl Output for Device {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn run(&self, running: bool) -> Result<(), String> {
        let result = if running {
            self.stream.play()
        } else {
            self.stream.pause()
        };
        result.map_err(|e| e.to_string())
    }
}

/// A stand-in for the device: the test asks for the sound itself.
#[cfg(test)]
pub struct Manual {
    render: std::sync::Mutex<Render>,
    rate: u32,
    running: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
impl Manual {
    pub fn new(render: Render, rate: u32) -> Self {
        Self {
            render: std::sync::Mutex::new(render),
            rate,
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// `frames` of sound, as the device would take them while running (silence when stopped).
    pub fn pull(&self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        if self.running.load(std::sync::atomic::Ordering::Relaxed)
            && let Ok(mut render) = self.render.lock()
        {
            render(&mut out);
        }
        out
    }
}

#[cfg(test)]
impl Output for std::sync::Arc<Manual> {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn run(&self, running: bool) -> Result<(), String> {
        self.running.store(running, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}
