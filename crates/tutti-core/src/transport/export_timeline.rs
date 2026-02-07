//! Export timeline for offline audio rendering.
//!
//! Provides a simulated transport that advances deterministically
//! based on sample count rather than real-time clock.

use crate::lockfree::{AtomicDouble, AtomicFlag, AtomicFloat};

/// Configuration for creating an export timeline.
#[derive(Debug, Clone)]
pub struct ExportConfig {
    /// Start position in beats.
    pub start_beat: f64,
    /// Tempo in BPM.
    pub tempo: f32,
    /// Sample rate in Hz.
    pub sample_rate: f64,
    /// Loop range (start, end) in beats, if looping.
    pub loop_range: Option<(f64, f64)>,
}

impl Default for ExportConfig {
    fn default() -> Self {
        Self {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        }
    }
}

/// Simulated transport timeline for offline export.
///
/// Implements `TransportReader` and advances beat position
/// based on sample count, not real-time.
///
/// # Example
/// ```ignore
/// let timeline = ExportTimeline::new(&ExportConfig {
///     start_beat: 0.0,
///     tempo: 120.0,
///     sample_rate: 44100.0,
///     loop_range: None,
/// });
///
/// // Advance by 44100 samples (1 second at 44.1kHz)
/// // At 120 BPM, that's 2 beats
/// timeline.advance(44100);
/// assert!((timeline.current_beat() - 2.0).abs() < 0.001);
/// ```
#[derive(Debug)]
pub struct ExportTimeline {
    /// Current position in beats.
    current_beat: AtomicDouble,
    /// Tempo in BPM.
    tempo: AtomicFloat,
    /// Sample rate in Hz.
    sample_rate: f64,
    /// Beats per sample (precomputed for efficiency).
    beats_per_sample: f64,
    /// Loop start in beats.
    loop_start: AtomicDouble,
    /// Loop end in beats.
    loop_end: AtomicDouble,
    /// Whether loop is enabled.
    loop_enabled: AtomicFlag,
}

impl ExportTimeline {
    /// Create a new export timeline with the given configuration.
    pub fn new(config: &ExportConfig) -> Self {
        let beats_per_second = config.tempo as f64 / 60.0;
        let beats_per_sample = beats_per_second / config.sample_rate;

        let (loop_start, loop_end, loop_enabled) = match config.loop_range {
            Some((start, end)) => (start, end, true),
            None => (0.0, 0.0, false),
        };

        Self {
            current_beat: AtomicDouble::new(config.start_beat),
            tempo: AtomicFloat::new(config.tempo),
            sample_rate: config.sample_rate,
            beats_per_sample,
            loop_start: AtomicDouble::new(loop_start),
            loop_end: AtomicDouble::new(loop_end),
            loop_enabled: AtomicFlag::new(loop_enabled),
        }
    }

    /// Advance the timeline by the given number of samples.
    ///
    /// If loop is enabled and the timeline crosses the loop end,
    /// it will wrap back to the loop start.
    pub fn advance(&self, samples: usize) {
        let mut beat = self.current_beat.get();
        beat += samples as f64 * self.beats_per_sample;

        // Handle loop wrap
        if self.loop_enabled.get() {
            let loop_start = self.loop_start.get();
            let loop_end = self.loop_end.get();

            if beat >= loop_end {
                let loop_length = loop_end - loop_start;
                if loop_length > 0.0 {
                    beat = loop_start + ((beat - loop_start) % loop_length);
                }
            }
        }

        self.current_beat.set(beat);
    }

    /// Get the current beat position.
    #[inline]
    pub fn current_beat(&self) -> f64 {
        self.current_beat.get()
    }

    /// Get the tempo in BPM.
    #[inline]
    pub fn tempo(&self) -> f32 {
        self.tempo.get()
    }

    /// Get the sample rate.
    #[inline]
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    /// Reset the timeline to the start beat.
    pub fn reset(&self, start_beat: f64) {
        self.current_beat.set(start_beat);
    }

    /// Get beats per sample (for calculating beat increment per tick).
    #[inline]
    pub fn beats_per_sample(&self) -> f64 {
        self.beats_per_sample
    }
}

impl super::TransportReader for ExportTimeline {
    fn current_beat(&self) -> f64 {
        self.current_beat.get()
    }

    fn is_loop_enabled(&self) -> bool {
        self.loop_enabled.get()
    }

    fn get_loop_range(&self) -> Option<(f64, f64)> {
        if self.loop_enabled.get() {
            Some((self.loop_start.get(), self.loop_end.get()))
        } else {
            None
        }
    }

    fn is_playing(&self) -> bool {
        // Export timeline is always "playing"
        true
    }

    fn is_recording(&self) -> bool {
        // Export timeline is never recording
        false
    }

    fn is_in_preroll(&self) -> bool {
        // Export timeline is never in preroll
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timeline_advances() {
        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        // At 120 BPM, 2 beats/second, 44100 samples/second
        // So 22050 samples = 1 beat
        timeline.advance(22050);
        assert!((timeline.current_beat() - 1.0).abs() < 0.001);

        timeline.advance(22050);
        assert!((timeline.current_beat() - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_timeline_loop_wrap() {
        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: Some((0.0, 4.0)),
        });

        // 4 beats at 120 BPM = 2 seconds = 88200 samples
        let samples_per_beat = 44100.0 / 2.0;

        // Advance to beat 3
        timeline.advance((3.0 * samples_per_beat) as usize);
        assert!((timeline.current_beat() - 3.0).abs() < 0.01);

        // Advance 2 more beats - should wrap to beat 1
        timeline.advance((2.0 * samples_per_beat) as usize);
        assert!((timeline.current_beat() - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_no_loop() {
        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        let samples_per_beat = 44100.0 / 2.0;

        // Advance past where loop end would be
        timeline.advance((10.0 * samples_per_beat) as usize);
        assert!((timeline.current_beat() - 10.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_start_offset() {
        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 4.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        assert!((timeline.current_beat() - 4.0).abs() < 0.001);

        let samples_per_beat = 44100.0 / 2.0;
        timeline.advance(samples_per_beat as usize);
        assert!((timeline.current_beat() - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_reset() {
        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        let samples_per_beat = 44100.0 / 2.0;
        timeline.advance((5.0 * samples_per_beat) as usize);
        assert!((timeline.current_beat() - 5.0).abs() < 0.01);

        // Reset to beat 2
        timeline.reset(2.0);
        assert!((timeline.current_beat() - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_transport_reader_impl() {
        use crate::TransportReader;

        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: Some((0.0, 8.0)),
        });

        // TransportReader methods
        assert!(timeline.is_playing());
        assert!(!timeline.is_recording());
        assert!(!timeline.is_in_preroll());
        assert!(timeline.is_loop_enabled());
        assert_eq!(timeline.get_loop_range(), Some((0.0, 8.0)));
    }

    #[test]
    fn test_transport_reader_no_loop() {
        use crate::TransportReader;

        let timeline = ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        assert!(!timeline.is_loop_enabled());
        assert_eq!(timeline.get_loop_range(), None);
    }

}
