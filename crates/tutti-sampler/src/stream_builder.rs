//! Stream and record builders for fluent API

use crate::system::{CaptureSession, SamplerSystem};
use std::path::PathBuf;

/// Builder for streaming audio files.
///
/// Created via `SamplerSystem::stream()`.
///
/// # Example
/// ```ignore
/// sampler.stream("long_audio.wav")
///     .channel(0)
///     .gain(0.8)
///     .speed(1.5)
///     .start_sample(44100)
///     .start();
/// ```
pub struct StreamBuilder<'a> {
    sampler: Option<&'a SamplerSystem>,
    file_path: PathBuf,
    channel: usize,
    start_sample: usize,
    duration_samples: usize,
    offset_samples: usize,
    speed: f32,
    gain: f32,
}

impl<'a> StreamBuilder<'a> {
    pub(crate) fn new(sampler: &'a SamplerSystem, file_path: impl Into<PathBuf>) -> Self {
        Self {
            sampler: Some(sampler),
            file_path: file_path.into(),
            channel: 0,
            start_sample: 0,
            duration_samples: usize::MAX,
            offset_samples: 0,
            speed: 1.0,
            gain: 1.0,
        }
    }

    /// Create a disabled builder (no-op when start() is called).
    pub(crate) fn disabled() -> Self {
        Self {
            sampler: None,
            file_path: PathBuf::new(),
            channel: 0,
            start_sample: 0,
            duration_samples: usize::MAX,
            offset_samples: 0,
            speed: 1.0,
            gain: 1.0,
        }
    }

    /// Set channel index for playback (default: 0).
    pub fn channel(mut self, index: usize) -> Self {
        self.channel = index;
        self
    }

    /// Set start sample position (default: 0).
    pub fn start_sample(mut self, sample: usize) -> Self {
        self.start_sample = sample;
        self
    }

    /// Set duration in samples (default: entire file).
    pub fn duration_samples(mut self, samples: usize) -> Self {
        self.duration_samples = samples;
        self
    }

    /// Set offset in samples (default: 0).
    pub fn offset_samples(mut self, offset: usize) -> Self {
        self.offset_samples = offset;
        self
    }

    /// Set playback speed multiplier (default: 1.0).
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Set gain/volume (default: 1.0).
    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Start streaming.
    ///
    /// No-op when sampler is disabled.
    pub fn start(self) {
        if let Some(sampler) = self.sampler {
            sampler
                .stream_file(self.channel, self.file_path)
                .start_sample(self.start_sample)
                .duration_samples(self.duration_samples)
                .offset_samples(self.offset_samples)
                .speed(self.speed)
                .gain(self.gain)
                .start();
        }
    }
}

/// Builder for recording audio.
///
/// Created via `SamplerSystem::record()`.
///
/// # Example
/// ```ignore
/// let session = sampler.record("output.wav")
///     .channels(2)
///     .buffer_seconds(5.0)
///     .start();
///
/// // Audio callback writes to session.producer
///
/// // Later...
/// sampler.stop_capture(session.id);
/// sampler.flush_capture(session.id, "final.wav");
/// ```
pub struct RecordBuilder<'a> {
    sampler: &'a SamplerSystem,
    file_path: PathBuf,
    channels: usize,
    buffer_seconds: Option<f64>,
    sample_rate: Option<f64>,
}

impl<'a> RecordBuilder<'a> {
    pub(crate) fn new(sampler: &'a SamplerSystem, file_path: impl Into<PathBuf>) -> Self {
        Self {
            sampler,
            file_path: file_path.into(),
            channels: 2,
            buffer_seconds: None,
            sample_rate: None,
        }
    }

    /// Set number of channels (default: 2).
    pub fn channels(mut self, channels: usize) -> Self {
        self.channels = channels;
        self
    }

    /// Set ring buffer size in seconds (default: 5.0).
    pub fn buffer_seconds(mut self, seconds: f64) -> Self {
        self.buffer_seconds = Some(seconds);
        self
    }

    /// Set sample rate (default: sampler system sample rate).
    pub fn sample_rate(mut self, rate: f64) -> Self {
        self.sample_rate = Some(rate);
        self
    }

    /// Start recording and return a capture session.
    ///
    /// The returned session contains a producer that the audio callback
    /// should write to. The butler thread will read from the ring buffer
    /// and write to disk asynchronously.
    pub fn start(self) -> CaptureSession {
        let sample_rate = self
            .sample_rate
            .unwrap_or_else(|| self.sampler.sample_rate());

        let session = self.sampler.create_capture(
            self.file_path,
            sample_rate,
            self.channels,
            self.buffer_seconds,
        );

        self.sampler.start_capture(session)
    }
}
