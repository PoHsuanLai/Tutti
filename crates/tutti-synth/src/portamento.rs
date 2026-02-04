//! Portamento (pitch glide) for synthesizers.
//!
//! Provides smooth pitch transitions between notes with configurable
//! glide time, curve shape, and legato-only mode.
//!
//! # Example
//!
//! ```ignore
//! use tutti_synth::portamento::{Portamento, PortamentoConfig, PortamentoMode};
//!
//! let config = PortamentoConfig {
//!     mode: PortamentoMode::Always,
//!     time: 0.1, // 100ms glide
//!     ..Default::default()
//! };
//!
//! let mut porta = Portamento::new(config, 44100.0);
//!
//! // On note on
//! porta.set_target(440.0, false); // A4
//!
//! // In audio loop
//! for _ in 0..buffer_size {
//!     let freq = porta.tick();
//!     // Use freq for oscillator
//! }
//! ```

/// Portamento/glide mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortamentoMode {
    /// Always glide between notes
    Always,
    /// Only glide during legato (overlapping notes)
    LegatoOnly,
    /// Disabled (no glide)
    #[default]
    Off,
}

/// Portamento curve shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PortamentoCurve {
    /// Linear interpolation
    #[default]
    Linear,
    /// Exponential curve (slow start, fast finish)
    Exponential,
    /// Logarithmic curve (fast start, slow finish)
    Logarithmic,
}

/// Configuration for portamento.
#[derive(Debug, Clone)]
pub struct PortamentoConfig {
    /// Glide mode
    pub mode: PortamentoMode,
    /// Curve shape
    pub curve: PortamentoCurve,
    /// Glide time in seconds
    pub time: f32,
    /// If true, glide time is constant regardless of interval.
    /// If false, larger intervals take proportionally longer.
    pub constant_time: bool,
}

impl Default for PortamentoConfig {
    fn default() -> Self {
        Self {
            mode: PortamentoMode::Off,
            curve: PortamentoCurve::Linear,
            time: 0.1,
            constant_time: true,
        }
    }
}

/// Per-voice portamento state.
///
/// Handles smooth pitch glide between notes.
/// All methods are RT-safe.
#[derive(Debug, Clone)]
pub struct Portamento {
    config: PortamentoConfig,
    /// Starting frequency in Hz
    start_freq: f32,
    /// Target frequency in Hz
    target_freq: f32,
    /// Current frequency in Hz
    current_freq: f32,
    /// Progress (0.0 to 1.0)
    progress: f32,
    /// Glide rate per sample
    rate: f32,
    /// Sample rate
    sample_rate: f32,
}

impl Portamento {
    /// Create a new portamento processor.
    pub fn new(config: PortamentoConfig, sample_rate: f32) -> Self {
        Self {
            config,
            start_freq: 440.0,
            target_freq: 440.0,
            current_freq: 440.0,
            progress: 1.0,
            rate: 0.0,
            sample_rate,
        }
    }

    /// Set the target frequency.
    ///
    /// # Arguments
    /// * `freq` - Target frequency in Hz
    /// * `is_legato` - True if this is a legato transition (overlapping notes)
    pub fn set_target(&mut self, freq: f32, is_legato: bool) {
        let should_glide = match self.config.mode {
            PortamentoMode::Off => false,
            PortamentoMode::Always => true,
            PortamentoMode::LegatoOnly => is_legato,
        };

        if should_glide && self.config.time > 0.0 {
            // Start glide from current position
            self.start_freq = self.current_freq;
            self.target_freq = freq;

            // Calculate glide time
            let glide_time = if self.config.constant_time {
                self.config.time
            } else {
                // Proportional: scale by interval (semitones / octave)
                let interval = (freq / self.start_freq).abs().log2().abs();
                self.config.time * (interval / 1.0).max(0.1) // At least 10% of base time
            };

            // Calculate rate
            let glide_samples = glide_time * self.sample_rate;
            self.rate = if glide_samples > 0.0 {
                1.0 / glide_samples
            } else {
                1.0
            };
            self.progress = 0.0;
        } else {
            // No glide: jump immediately
            self.start_freq = freq;
            self.target_freq = freq;
            self.current_freq = freq;
            self.progress = 1.0;
        }
    }

    /// Process one sample and return current frequency.
    ///
    /// RT-safe.
    #[inline]
    pub fn tick(&mut self) -> f32 {
        if self.progress >= 1.0 {
            return self.target_freq;
        }

        self.progress += self.rate;
        self.progress = self.progress.min(1.0);

        // Apply curve shape
        let t = match self.config.curve {
            PortamentoCurve::Linear => self.progress,
            PortamentoCurve::Exponential => self.progress * self.progress,
            PortamentoCurve::Logarithmic => self.progress.sqrt(),
        };

        // Interpolate in log space for musical pitch glide
        let log_start = self.start_freq.ln();
        let log_target = self.target_freq.ln();
        self.current_freq = (log_start + (log_target - log_start) * t).exp();

        self.current_freq
    }

    /// Get current frequency without advancing.
    #[inline]
    pub fn current(&self) -> f32 {
        self.current_freq
    }

    /// Get target frequency.
    #[inline]
    pub fn target(&self) -> f32 {
        self.target_freq
    }

    /// Check if glide is complete.
    #[inline]
    pub fn is_complete(&self) -> bool {
        self.progress >= 1.0
    }

    /// Get glide progress (0.0 to 1.0).
    #[inline]
    pub fn progress(&self) -> f32 {
        self.progress
    }

    /// Reset to a specific frequency (no glide).
    pub fn reset(&mut self, freq: f32) {
        self.start_freq = freq;
        self.target_freq = freq;
        self.current_freq = freq;
        self.progress = 1.0;
    }

    /// Update configuration.
    pub fn set_config(&mut self, config: PortamentoConfig) {
        self.config = config;
    }

    /// Get current configuration.
    pub fn config(&self) -> &PortamentoConfig {
        &self.config
    }

    /// Set sample rate.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
    }

    /// Set glide time.
    pub fn set_time(&mut self, time: f32) {
        self.config.time = time;
    }

    /// Set glide mode.
    pub fn set_mode(&mut self, mode: PortamentoMode) {
        self.config.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_glide() {
        let config = PortamentoConfig {
            mode: PortamentoMode::Off,
            ..Default::default()
        };
        let mut porta = Portamento::new(config, 44100.0);

        porta.set_target(880.0, false);
        assert!((porta.tick() - 880.0).abs() < 0.01);
        assert!(porta.is_complete());
    }

    #[test]
    fn test_always_glide() {
        let config = PortamentoConfig {
            mode: PortamentoMode::Always,
            time: 0.01, // 10ms
            ..Default::default()
        };
        let mut porta = Portamento::new(config, 44100.0);

        porta.reset(440.0);
        porta.set_target(880.0, false);

        // Should start near 440
        let first = porta.tick();
        assert!(first < 500.0);

        // Run until complete
        while !porta.is_complete() {
            porta.tick();
        }

        // Should end at 880
        assert!((porta.current() - 880.0).abs() < 1.0);
    }

    #[test]
    fn test_legato_only() {
        let config = PortamentoConfig {
            mode: PortamentoMode::LegatoOnly,
            time: 0.01,
            ..Default::default()
        };
        let mut porta = Portamento::new(config, 44100.0);

        porta.reset(440.0);

        // Non-legato should not glide
        porta.set_target(880.0, false);
        assert!(porta.is_complete());
        assert!((porta.current() - 880.0).abs() < 0.01);

        // Legato should glide
        porta.set_target(440.0, true);
        assert!(!porta.is_complete());
    }

    #[test]
    fn test_curve_shapes() {
        for curve in [
            PortamentoCurve::Linear,
            PortamentoCurve::Exponential,
            PortamentoCurve::Logarithmic,
        ] {
            let config = PortamentoConfig {
                mode: PortamentoMode::Always,
                curve,
                time: 0.01,
                ..Default::default()
            };
            let mut porta = Portamento::new(config, 44100.0);

            porta.reset(440.0);
            porta.set_target(880.0, false);

            // Run to completion
            while !porta.is_complete() {
                porta.tick();
            }

            // All curves should reach the target
            assert!((porta.current() - 880.0).abs() < 1.0);
        }
    }
}
