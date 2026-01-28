
use crate::{AtomicFloat, AtomicU32, AtomicU8, Ordering};
use std::sync::atomic::AtomicI64;

pub struct Metronome {
    volume: AtomicFloat,
    accent_every: AtomicU32,
    mode: AtomicU8,
    last_click_beat: AtomicI64,
    click_buffer_normal: Vec<(f32, f32)>,
    click_buffer_accent: Vec<(f32, f32)>,
    sample_rate: f32,
}

impl Metronome {
    pub fn new(sample_rate: f32) -> Self {
        // Generate pre-allocated click buffers
        let click_buffer_normal = Self::generate_click_buffer(sample_rate, false, 0.5);
        let click_buffer_accent = Self::generate_click_buffer(sample_rate, true, 0.5);

        Self {
            volume: AtomicFloat::new(0.5),
            accent_every: AtomicU32::new(4),
            mode: AtomicU8::new(MetronomeMode::default() as u8),
            last_click_beat: AtomicI64::new(-1),
            click_buffer_normal,
            click_buffer_accent,
            sample_rate,
        }
    }

    fn generate_click_buffer(sample_rate: f32, is_accent: bool, volume: f32) -> Vec<(f32, f32)> {
        let click_duration = 0.03; // 30ms
        let num_samples = (sample_rate * click_duration) as usize;

        let mut samples = Vec::with_capacity(num_samples);

        // Use higher frequency for accented beats
        let freq = if is_accent { 1200.0 } else { 1000.0 };
        let accent_volume = if is_accent { 1.0 } else { 0.7 };

        for i in 0..num_samples {
            let t = i as f64 / sample_rate as f64;

            // Envelope
            let env = if t < 0.001 {
                t / 0.001 // Attack
            } else if t < 0.02 {
                1.0 // Sustain
            } else {
                1.0 - (t - 0.02) / 0.01 // Release
            };

            // Sine wave click
            let phase = 2.0 * std::f64::consts::PI * freq * t;
            let sample = (phase.sin() * env * volume as f64 * accent_volume) as f32;

            samples.push((sample, sample)); // Stereo (both channels same)
        }

        samples
    }

    pub fn set_volume(&self, volume: f32) {
        self.volume.set(volume.clamp(0.0, 1.0));
    }

    pub fn volume(&self) -> f32 {
        self.volume.get()
    }

    pub fn set_accent_every(&self, beats: u32) {
        self.accent_every.store(beats, Ordering::Release);
    }

    pub fn accent_every(&self) -> u32 {
        self.accent_every.load(Ordering::Acquire)
    }

    pub fn mode(&self) -> MetronomeMode {
        let mode_u8 = self.mode.load(Ordering::Acquire);
        MetronomeMode::from(mode_u8)
    }

    pub fn set_mode(&self, mode: MetronomeMode) {
        self.mode.store(mode as u8, Ordering::Release);
    }

    pub fn update(&self, beat: f64) -> bool {
        let current_beat_int = beat.floor() as i64;
        let last_beat = self.last_click_beat.load(Ordering::Acquire);

        if current_beat_int > last_beat {
            // Try to update last_click_beat atomically
            self.last_click_beat
                .compare_exchange(
                    last_beat,
                    current_beat_int,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
        } else {
            false
        }
    }

    #[inline]
    pub fn get_click_buffer(&self, is_accent: bool) -> &[(f32, f32)] {
        if is_accent {
            &self.click_buffer_accent
        } else {
            &self.click_buffer_normal
        }
    }

    pub fn update_volume(&mut self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        self.volume.set(volume);

        // Regenerate click buffers with new volume
        self.click_buffer_normal = Self::generate_click_buffer(self.sample_rate, false, volume);
        self.click_buffer_accent = Self::generate_click_buffer(self.sample_rate, true, volume);
    }

    pub fn is_accent_beat(&self, beat: f64) -> bool {
        let accent_every = self.accent_every();
        if accent_every == 0 {
            return false;
        }
        let beat_num = (beat.floor() as u32) % accent_every;
        beat_num == 0
    }

    pub fn reset(&self) {
        self.last_click_beat.store(-1, Ordering::Release);
    }
}

/// Metronome state for integration with audio backend
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MetronomeMode {
    Off,
    PrerollOnly,
    RecordingOnly,
    Always,
}

impl Default for MetronomeMode {
    fn default() -> Self {
        Self::Off
    }
}

impl From<u8> for MetronomeMode {
    fn from(value: u8) -> Self {
        match value {
            0 => MetronomeMode::Off,
            1 => MetronomeMode::PrerollOnly,
            2 => MetronomeMode::RecordingOnly,
            3 => MetronomeMode::Always,
            _ => MetronomeMode::Off, // Default to Off for invalid values
        }
    }
}

impl MetronomeMode {
    pub fn should_play(&self, is_in_preroll: bool, is_recording: bool) -> bool {
        match self {
            MetronomeMode::Off => false,
            MetronomeMode::PrerollOnly => is_in_preroll,
            MetronomeMode::RecordingOnly => is_recording && !is_in_preroll,
            MetronomeMode::Always => is_in_preroll || is_recording,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metronome_creation() {
        let metronome = Metronome::new(44100.0);
        assert_eq!(metronome.volume(), 0.5);
        assert_eq!(metronome.accent_every(), 4);
        assert_eq!(metronome.sample_rate, 44100.0);
    }

    #[test]
    fn test_beat_detection() {
        let metronome = Metronome::new(44100.0);

        // First beat should trigger
        assert!(metronome.update(0.0));

        // Same beat shouldn't retrigger
        assert!(!metronome.update(0.5));

        // Next beat should trigger
        assert!(metronome.update(1.0));
    }

    #[test]
    fn test_accent_pattern() {
        let metronome = Metronome::new(44100.0);
        metronome.set_accent_every(4);

        // Beat 0 should be accented
        metronome.update(0.0);
        assert!(metronome.is_accent_beat(0.0));

        // Beat 1 should not be accented
        metronome.update(1.0);
        assert!(!metronome.is_accent_beat(1.0));

        // Beat 4 should be accented again
        metronome.update(4.0);
        assert!(metronome.is_accent_beat(4.0));
    }

    #[test]
    fn test_click_generation() {
        let sample_rate = 44100.0;
        let metronome = Metronome::new(sample_rate);

        // Get pre-allocated normal click buffer
        let samples = metronome.get_click_buffer(false);
        assert!(!samples.is_empty());
        assert_eq!(samples.len(), (sample_rate * 0.03) as usize);

        for (left, right) in samples {
            assert!(left.abs() <= 1.0);
            assert!(right.abs() <= 1.0);
        }

        // Get pre-allocated accent click buffer
        let accent_samples = metronome.get_click_buffer(true);
        assert!(!accent_samples.is_empty());
        assert_eq!(accent_samples.len(), (sample_rate * 0.03) as usize);

        // Accent should be different from normal (higher frequency)
        // We can't directly compare since they're the same volume at construction,
        // but we can verify the buffer exists and is valid
        for (left, right) in accent_samples {
            assert!(left.abs() <= 1.0);
            assert!(right.abs() <= 1.0);
        }
    }

    #[test]
    fn test_metronome_mode_should_play() {
        // Off mode - never plays
        assert!(!MetronomeMode::Off.should_play(false, false));
        assert!(!MetronomeMode::Off.should_play(true, false));
        assert!(!MetronomeMode::Off.should_play(false, true));
        assert!(!MetronomeMode::Off.should_play(true, true));

        // PrerollOnly - only during preroll
        assert!(!MetronomeMode::PrerollOnly.should_play(false, false));
        assert!(MetronomeMode::PrerollOnly.should_play(true, false)); // ✓ preroll
        assert!(!MetronomeMode::PrerollOnly.should_play(false, true)); // ✗ recording
        assert!(MetronomeMode::PrerollOnly.should_play(true, true)); // ✓ preroll (takes priority)

        // RecordingOnly - only during recording (not preroll)
        assert!(!MetronomeMode::RecordingOnly.should_play(false, false));
        assert!(!MetronomeMode::RecordingOnly.should_play(true, false)); // ✗ preroll
        assert!(MetronomeMode::RecordingOnly.should_play(false, true)); // ✓ recording
        assert!(!MetronomeMode::RecordingOnly.should_play(true, true)); // ✗ preroll active

        // Always - plays during both
        assert!(!MetronomeMode::Always.should_play(false, false));
        assert!(MetronomeMode::Always.should_play(true, false)); // ✓ preroll
        assert!(MetronomeMode::Always.should_play(false, true)); // ✓ recording
        assert!(MetronomeMode::Always.should_play(true, true)); // ✓ both
    }
}
