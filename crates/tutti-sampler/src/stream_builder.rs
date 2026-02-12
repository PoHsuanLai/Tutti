//! Stream and record builders for fluent API

use crate::butler::PlayDirection;
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
///     .offset_samples(44100)
///     .loop_samples(0, 88200)
///     .crossfade_samples(256)
///     .start();
/// ```
pub struct StreamBuilder<'a> {
    sampler: Option<&'a SamplerSystem>,
    file_path: PathBuf,
    channel: usize,
    offset_samples: usize,
    loop_range: Option<(u64, u64)>,
    crossfade_samples: usize,
    direction: PlayDirection,
    speed: f32,
}

impl<'a> StreamBuilder<'a> {
    pub(crate) fn new(sampler: &'a SamplerSystem, file_path: impl Into<PathBuf>) -> Self {
        Self {
            sampler: Some(sampler),
            file_path: file_path.into(),
            channel: 0,
            offset_samples: 0,
            loop_range: None,
            crossfade_samples: 0,
            direction: PlayDirection::Forward,
            speed: 1.0,
        }
    }

    /// Create a disabled builder (no-op when start() is called).
    pub(crate) fn disabled() -> Self {
        Self {
            sampler: None,
            file_path: PathBuf::new(),
            channel: 0,
            offset_samples: 0,
            loop_range: None,
            crossfade_samples: 0,
            direction: PlayDirection::Forward,
            speed: 1.0,
        }
    }

    /// Set channel index for playback (default: 0).
    pub fn channel(mut self, index: usize) -> Self {
        self.channel = index;
        self
    }

    /// Set offset in samples (default: 0).
    pub fn offset_samples(mut self, offset: usize) -> Self {
        self.offset_samples = offset;
        self
    }

    /// Set loop range in samples.
    pub fn loop_samples(mut self, start: u64, end: u64) -> Self {
        self.loop_range = Some((start, end));
        self
    }

    /// Set crossfade length for smooth loop transitions (default: 0 = no crossfade).
    pub fn crossfade_samples(mut self, samples: usize) -> Self {
        self.crossfade_samples = samples;
        self
    }

    /// Enable reverse playback.
    pub fn reverse(mut self) -> Self {
        self.direction = PlayDirection::Reverse;
        self
    }

    /// Set playback speed (1.0 = normal, negative = reverse).
    pub fn speed(mut self, speed: f32) -> Self {
        if speed < 0.0 {
            self.direction = PlayDirection::Reverse;
            self.speed = speed.abs();
        } else {
            self.speed = speed;
        }
        self
    }

    /// Start streaming.
    ///
    /// No-op when sampler is disabled.
    pub fn start(self) {
        let Some(sampler) = self.sampler else {
            return;
        };

        // Send the stream command to butler
        sampler.send_stream_command(self.channel, self.file_path, self.offset_samples);

        // Apply loop range if set
        if let Some((start, end)) = self.loop_range {
            sampler.set_loop_range_with_crossfade(self.channel, start, end, self.crossfade_samples);
        }

        // Apply speed/direction if non-default
        if self.direction == PlayDirection::Reverse || (self.speed - 1.0).abs() > 0.001 {
            sampler.set_speed(
                self.channel,
                if self.direction == PlayDirection::Reverse {
                    -self.speed
                } else {
                    self.speed
                },
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // StreamBuilder tests (disabled path - no SamplerSystem needed)
    // =========================================================================

    #[test]
    fn test_stream_builder_disabled_defaults() {
        let builder = StreamBuilder::disabled();

        assert!(builder.sampler.is_none());
        assert_eq!(builder.file_path, PathBuf::new());
        assert_eq!(builder.channel, 0);
        assert_eq!(builder.offset_samples, 0);
        assert!(builder.loop_range.is_none());
        assert_eq!(builder.crossfade_samples, 0);
        assert_eq!(builder.direction, PlayDirection::Forward);
        assert_eq!(builder.speed, 1.0);
    }

    #[test]
    fn test_stream_builder_disabled_start_is_noop() {
        // Should not panic or do anything
        let builder = StreamBuilder::disabled();
        builder.start();
    }

    #[test]
    fn test_stream_builder_channel() {
        let builder = StreamBuilder::disabled().channel(5);
        assert_eq!(builder.channel, 5);
    }

    #[test]
    fn test_stream_builder_offset_samples() {
        let builder = StreamBuilder::disabled().offset_samples(44100);
        assert_eq!(builder.offset_samples, 44100);
    }

    #[test]
    fn test_stream_builder_loop_samples() {
        let builder = StreamBuilder::disabled().loop_samples(1000, 5000);
        assert_eq!(builder.loop_range, Some((1000, 5000)));
    }

    #[test]
    fn test_stream_builder_crossfade_samples() {
        let builder = StreamBuilder::disabled().crossfade_samples(256);
        assert_eq!(builder.crossfade_samples, 256);
    }

    #[test]
    fn test_stream_builder_reverse() {
        let builder = StreamBuilder::disabled().reverse();
        assert_eq!(builder.direction, PlayDirection::Reverse);
    }

    #[test]
    fn test_stream_builder_speed_positive() {
        let builder = StreamBuilder::disabled().speed(2.0);
        assert_eq!(builder.speed, 2.0);
        assert_eq!(builder.direction, PlayDirection::Forward);
    }

    #[test]
    fn test_stream_builder_speed_negative_sets_reverse() {
        let builder = StreamBuilder::disabled().speed(-1.5);
        assert_eq!(builder.speed, 1.5); // abs value
        assert_eq!(builder.direction, PlayDirection::Reverse);
    }

    #[test]
    fn test_stream_builder_chaining() {
        let builder = StreamBuilder::disabled()
            .channel(2)
            .offset_samples(1000)
            .loop_samples(0, 10000)
            .crossfade_samples(128)
            .speed(1.5);

        assert_eq!(builder.channel, 2);
        assert_eq!(builder.offset_samples, 1000);
        assert_eq!(builder.loop_range, Some((0, 10000)));
        assert_eq!(builder.crossfade_samples, 128);
        assert_eq!(builder.speed, 1.5);
    }

    #[test]
    fn test_stream_builder_chained_start_is_noop() {
        // Full chain with disabled builder should still be safe
        StreamBuilder::disabled()
            .channel(0)
            .offset_samples(44100)
            .loop_samples(0, 88200)
            .crossfade_samples(256)
            .reverse()
            .start();
    }
}
