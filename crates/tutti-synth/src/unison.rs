//! Unison engine for synthesizers.
//!
//! Provides voice detuning and stereo spread for thicker sounds.
//! Pre-computes all voice parameters for RT-safe per-sample lookup.
//!
//! # Example
//!
//! ```ignore
//! use tutti_synth::unison::{UnisonEngine, UnisonConfig};
//!
//! let config = UnisonConfig {
//!     voice_count: 7,
//!     detune_cents: 15.0,
//!     stereo_spread: 1.0,
//!     ..Default::default()
//! };
//!
//! let mut unison = UnisonEngine::new(config);
//!
//! // On note on, optionally randomize phases
//! unison.randomize_phases();
//!
//! // Get parameters for each unison voice
//! for i in 0..unison.voice_count() {
//!     let params = unison.voice_params(i);
//!     // Use params.freq_ratio, params.pan, params.amplitude for oscillator
//! }
//! ```

/// Maximum unison voices supported.
pub const MAX_UNISON_VOICES: usize = 16;

/// Unison configuration.
#[derive(Debug, Clone)]
pub struct UnisonConfig {
    /// Number of unison voices (1-16)
    pub voice_count: u8,
    /// Detune spread in cents (total spread, not per-voice)
    pub detune_cents: f32,
    /// Stereo spread (0.0 = mono, 1.0 = full stereo)
    pub stereo_spread: f32,
    /// Randomize phase on note-on
    pub phase_randomize: bool,
}

impl Default for UnisonConfig {
    fn default() -> Self {
        Self {
            voice_count: 1,
            detune_cents: 0.0,
            stereo_spread: 0.0,
            phase_randomize: false,
        }
    }
}

/// Pre-computed parameters for a single unison voice.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnisonVoiceParams {
    /// Frequency multiplier (1.0 = center pitch)
    pub freq_ratio: f32,
    /// Pan position (-1.0 = left, 1.0 = right)
    pub pan: f32,
    /// Phase offset (0.0 to 1.0)
    pub phase_offset: f32,
    /// Amplitude (equal-power scaled)
    pub amplitude: f32,
}

/// Unison engine for detuned voice stacking.
///
/// Pre-computes frequency ratios, pan positions, and amplitudes
/// for efficient RT-safe per-sample use.
#[derive(Debug, Clone)]
pub struct UnisonEngine {
    config: UnisonConfig,
    /// Pre-computed voice parameters
    voices: [UnisonVoiceParams; MAX_UNISON_VOICES],
    /// Simple RNG state for phase randomization
    rng_state: u32,
}

impl UnisonEngine {
    /// Create a new unison engine.
    pub fn new(config: UnisonConfig) -> Self {
        let mut engine = Self {
            config,
            voices: [UnisonVoiceParams::default(); MAX_UNISON_VOICES],
            rng_state: 12345,
        };
        engine.recompute_params();
        engine
    }

    /// Recompute all voice parameters based on current config.
    ///
    /// Call this when config changes.
    pub fn recompute_params(&mut self) {
        let count = (self.config.voice_count as usize).clamp(1, MAX_UNISON_VOICES);

        // Equal-power amplitude scaling
        let amplitude = 1.0 / (count as f32).sqrt();

        // Detune spread in semitones
        let detune_semitones = self.config.detune_cents / 100.0;

        for i in 0..count {
            // Spread voices evenly across detune range
            // For odd count: center voice at 0, others spread symmetrically
            // For even count: spread symmetrically with no center
            let position = if count == 1 {
                0.0
            } else {
                // Position from -1.0 to 1.0
                (i as f32 / (count - 1) as f32) * 2.0 - 1.0
            };

            // Frequency ratio from detune (in log space)
            // semitones * position / 12 gives octave fraction
            let freq_ratio = 2.0_f32.powf(detune_semitones * position / 12.0);

            // Pan position (spread from center)
            let pan = position * self.config.stereo_spread;

            self.voices[i] = UnisonVoiceParams {
                freq_ratio,
                pan,
                phase_offset: 0.0,
                amplitude,
            };
        }

        // Clear unused voices
        for i in count..MAX_UNISON_VOICES {
            self.voices[i] = UnisonVoiceParams::default();
        }
    }

    /// Randomize phase offsets for all voices.
    ///
    /// Call on note-on for natural detuned sound.
    /// Uses simple xorshift for RT-safety (no allocations).
    pub fn randomize_phases(&mut self) {
        if !self.config.phase_randomize {
            return;
        }

        let count = (self.config.voice_count as usize).clamp(1, MAX_UNISON_VOICES);

        for i in 0..count {
            // Simple xorshift32 RNG
            self.rng_state ^= self.rng_state << 13;
            self.rng_state ^= self.rng_state >> 17;
            self.rng_state ^= self.rng_state << 5;

            // Convert to 0.0-1.0 range
            self.voices[i].phase_offset = (self.rng_state as f32) / (u32::MAX as f32);
        }
    }

    /// Get the number of active unison voices.
    #[inline]
    pub fn voice_count(&self) -> usize {
        (self.config.voice_count as usize).clamp(1, MAX_UNISON_VOICES)
    }

    /// Get parameters for a specific voice.
    ///
    /// RT-safe: just an array lookup.
    #[inline]
    pub fn voice_params(&self, index: usize) -> &UnisonVoiceParams {
        &self.voices[index.min(MAX_UNISON_VOICES - 1)]
    }

    /// Get all voice parameters as a slice.
    #[inline]
    pub fn all_params(&self) -> &[UnisonVoiceParams] {
        &self.voices[..self.voice_count()]
    }

    /// Update configuration and recompute parameters.
    pub fn set_config(&mut self, config: UnisonConfig) {
        self.config = config;
        self.recompute_params();
    }

    /// Get current configuration.
    pub fn config(&self) -> &UnisonConfig {
        &self.config
    }

    /// Set voice count.
    pub fn set_voice_count(&mut self, count: u8) {
        self.config.voice_count = count.clamp(1, MAX_UNISON_VOICES as u8);
        self.recompute_params();
    }

    /// Set detune amount in cents.
    pub fn set_detune(&mut self, cents: f32) {
        self.config.detune_cents = cents.max(0.0);
        self.recompute_params();
    }

    /// Set stereo spread.
    pub fn set_stereo_spread(&mut self, spread: f32) {
        self.config.stereo_spread = spread.clamp(0.0, 1.0);
        self.recompute_params();
    }

    /// Seed the RNG for reproducible phase randomization.
    pub fn seed_rng(&mut self, seed: u32) {
        self.rng_state = if seed == 0 { 1 } else { seed };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_voice() {
        let config = UnisonConfig {
            voice_count: 1,
            ..Default::default()
        };
        let unison = UnisonEngine::new(config);

        assert_eq!(unison.voice_count(), 1);

        let params = unison.voice_params(0);
        assert!((params.freq_ratio - 1.0).abs() < 0.001);
        assert!((params.pan - 0.0).abs() < 0.001);
        assert!((params.amplitude - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_detune_spread() {
        let config = UnisonConfig {
            voice_count: 3,
            detune_cents: 12.0, // ~1/8 semitone spread
            stereo_spread: 0.0,
            phase_randomize: false,
        };
        let unison = UnisonEngine::new(config);

        assert_eq!(unison.voice_count(), 3);

        // Center voice should be at ratio 1.0
        let center = unison.voice_params(1);
        assert!((center.freq_ratio - 1.0).abs() < 0.001);

        // First voice should be detuned down
        let low = unison.voice_params(0);
        assert!(low.freq_ratio < 1.0);

        // Last voice should be detuned up
        let high = unison.voice_params(2);
        assert!(high.freq_ratio > 1.0);

        // Symmetric detune
        let low_ratio = 1.0 / low.freq_ratio;
        let high_ratio = high.freq_ratio;
        assert!((low_ratio - high_ratio).abs() < 0.001);
    }

    #[test]
    fn test_stereo_spread() {
        let config = UnisonConfig {
            voice_count: 3,
            detune_cents: 0.0,
            stereo_spread: 1.0,
            phase_randomize: false,
        };
        let unison = UnisonEngine::new(config);

        let left = unison.voice_params(0);
        let center = unison.voice_params(1);
        let right = unison.voice_params(2);

        assert!((left.pan - (-1.0)).abs() < 0.001);
        assert!((center.pan - 0.0).abs() < 0.001);
        assert!((right.pan - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_equal_power_amplitude() {
        for count in 1..=8 {
            let config = UnisonConfig {
                voice_count: count,
                ..Default::default()
            };
            let unison = UnisonEngine::new(config);

            // Sum of squared amplitudes should equal 1.0 (equal power)
            let sum_sq: f32 = unison
                .all_params()
                .iter()
                .map(|p| p.amplitude * p.amplitude)
                .sum();

            assert!(
                (sum_sq - 1.0).abs() < 0.01,
                "Equal power failed for {} voices: {}",
                count,
                sum_sq
            );
        }
    }

    #[test]
    fn test_phase_randomization() {
        let config = UnisonConfig {
            voice_count: 4,
            phase_randomize: true,
            ..Default::default()
        };
        let mut unison = UnisonEngine::new(config);

        // Seed for reproducibility
        unison.seed_rng(42);
        unison.randomize_phases();

        // Phases should be different
        let phases: Vec<f32> = unison.all_params().iter().map(|p| p.phase_offset).collect();

        // Check phases are in valid range
        for phase in &phases {
            assert!(*phase >= 0.0 && *phase <= 1.0);
        }

        // Check they're not all the same
        let first = phases[0];
        let all_same = phases.iter().all(|p| (*p - first).abs() < 0.001);
        assert!(!all_same, "Phases should be randomized");
    }

    #[test]
    fn test_config_update() {
        let mut unison = UnisonEngine::new(UnisonConfig::default());

        assert_eq!(unison.voice_count(), 1);

        unison.set_voice_count(5);
        assert_eq!(unison.voice_count(), 5);

        unison.set_detune(20.0);
        assert!((unison.config().detune_cents - 20.0).abs() < 0.001);

        unison.set_stereo_spread(0.5);
        assert!((unison.config().stereo_spread - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_clamp_voice_count() {
        let config = UnisonConfig {
            voice_count: 100, // Over max
            ..Default::default()
        };
        let unison = UnisonEngine::new(config);

        assert_eq!(unison.voice_count(), MAX_UNISON_VOICES);
    }
}
