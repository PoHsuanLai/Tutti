//! Fluent builder API for creating synthesizers.
//!
//! Combines tutti-synth building blocks with FunDSP primitives into
//! complete polyphonic synthesizers.

mod polysynth;
mod voice;

pub use polysynth::PolySynth;

use crate::{
    AllocationStrategy, ModulationMatrixConfig, PortamentoConfig, Tuning, UnisonConfig, VoiceMode,
};

/// Oscillator types wrapping FunDSP oscillators.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OscillatorType {
    /// Sine wave oscillator
    Sine,
    /// Sawtooth wave oscillator (band-limited)
    Saw,
    /// Square/pulse wave oscillator with configurable pulse width
    Square { pulse_width: f32 },
    /// Triangle wave oscillator
    Triangle,
    /// White noise generator
    Noise,
}

impl Default for OscillatorType {
    fn default() -> Self {
        Self::Saw
    }
}

/// SVF (State Variable Filter) modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SvfMode {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
}

/// Biquad filter modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BiquadMode {
    #[default]
    Lowpass,
    Highpass,
    Bandpass,
    Peak,
    Notch,
}

/// Filter types wrapping FunDSP filters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    /// Moog ladder filter (4-pole lowpass with resonance)
    Moog { cutoff: f32, resonance: f32 },
    /// State Variable Filter with selectable mode
    Svf { cutoff: f32, q: f32, mode: SvfMode },
    /// Biquad filter with selectable mode
    Biquad {
        cutoff: f32,
        q: f32,
        mode: BiquadMode,
    },
    /// No filter (bypass)
    None,
}

impl Default for FilterType {
    fn default() -> Self {
        Self::None
    }
}

/// ADSR envelope configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnvelopeConfig {
    /// Attack time in seconds
    pub attack: f32,
    /// Decay time in seconds
    pub decay: f32,
    /// Sustain level (0.0 - 1.0)
    pub sustain: f32,
    /// Release time in seconds
    pub release: f32,
}

impl Default for EnvelopeConfig {
    fn default() -> Self {
        Self {
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.2,
        }
    }
}

impl EnvelopeConfig {
    /// Create a new envelope configuration.
    pub fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self {
            attack,
            decay,
            sustain,
            release,
        }
    }

    /// Quick attack, no decay, full sustain.
    pub fn organ() -> Self {
        Self::new(0.001, 0.0, 1.0, 0.01)
    }

    /// Plucky envelope with fast attack and decay.
    pub fn pluck() -> Self {
        Self::new(0.001, 0.3, 0.0, 0.1)
    }

    /// Pad-style envelope with slow attack and release.
    pub fn pad() -> Self {
        Self::new(0.5, 0.2, 0.8, 1.0)
    }
}

/// Complete synth configuration.
#[derive(Debug, Clone)]
pub struct SynthConfig {
    /// Sample rate in Hz
    pub sample_rate: f64,
    /// Maximum number of voices
    pub max_voices: usize,
    /// Voice mode (poly/mono/legato)
    pub voice_mode: VoiceMode,
    /// Oscillator type
    pub oscillator: OscillatorType,
    /// Filter type
    pub filter: FilterType,
    /// ADSR envelope
    pub envelope: EnvelopeConfig,
    /// Portamento configuration (optional)
    pub portamento: Option<PortamentoConfig>,
    /// Unison configuration (optional)
    pub unison: Option<UnisonConfig>,
    /// Voice stealing strategy
    pub allocation_strategy: AllocationStrategy,
    /// Tuning system
    pub tuning: Tuning,
    /// Modulation matrix configuration (optional)
    pub modulation: Option<ModulationMatrixConfig>,
}

impl Default for SynthConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100.0,
            max_voices: 8,
            voice_mode: VoiceMode::Poly,
            oscillator: OscillatorType::default(),
            filter: FilterType::default(),
            envelope: EnvelopeConfig::default(),
            portamento: None,
            unison: None,
            allocation_strategy: AllocationStrategy::Oldest,
            tuning: Tuning::equal_temperament(),
            modulation: None,
        }
    }
}

/// Fluent builder for creating synthesizers.
#[derive(Debug, Clone)]
pub struct SynthBuilder {
    config: SynthConfig,
}

impl SynthBuilder {
    /// Create a new synth builder with the given sample rate.
    pub fn new(sample_rate: f64) -> Self {
        Self {
            config: SynthConfig {
                sample_rate,
                ..Default::default()
            },
        }
    }

    /// Set polyphonic mode with the specified number of voices.
    pub fn poly(mut self, voices: usize) -> Self {
        self.config.max_voices = voices;
        self.config.voice_mode = VoiceMode::Poly;
        self
    }

    /// Set monophonic mode (single voice).
    pub fn mono(mut self) -> Self {
        self.config.max_voices = 1;
        self.config.voice_mode = VoiceMode::Mono;
        self
    }

    /// Set legato mode (monophonic with pitch glide on overlapping notes).
    pub fn legato(mut self) -> Self {
        self.config.max_voices = 1;
        self.config.voice_mode = VoiceMode::Legato;
        self
    }

    /// Set the oscillator type.
    pub fn oscillator(mut self, osc: OscillatorType) -> Self {
        self.config.oscillator = osc;
        self
    }

    /// Set the filter type.
    pub fn filter(mut self, filter: FilterType) -> Self {
        self.config.filter = filter;
        self
    }

    /// Set ADSR envelope parameters.
    pub fn envelope(mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        self.config.envelope = EnvelopeConfig::new(attack, decay, sustain, release);
        self
    }

    /// Set envelope from a preset configuration.
    pub fn envelope_config(mut self, config: EnvelopeConfig) -> Self {
        self.config.envelope = config;
        self
    }

    /// Enable portamento with the given configuration.
    pub fn portamento(mut self, config: PortamentoConfig) -> Self {
        self.config.portamento = Some(config);
        self
    }

    /// Enable unison with the given configuration.
    pub fn unison(mut self, config: UnisonConfig) -> Self {
        self.config.unison = Some(config);
        self
    }

    /// Set the voice stealing strategy.
    pub fn voice_stealing(mut self, strategy: AllocationStrategy) -> Self {
        self.config.allocation_strategy = strategy;
        self
    }

    /// Set the tuning system.
    pub fn tuning(mut self, tuning: Tuning) -> Self {
        self.config.tuning = tuning;
        self
    }

    /// Enable modulation matrix with the given configuration.
    pub fn modulation(mut self, config: ModulationMatrixConfig) -> Self {
        self.config.modulation = Some(config);
        self
    }

    /// Build the synthesizer.
    pub fn build(self) -> crate::Result<PolySynth> {
        PolySynth::from_config(self.config)
    }

    /// Get the current configuration (for inspection).
    pub fn config(&self) -> &SynthConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_defaults() {
        let builder = SynthBuilder::new(44100.0);
        let config = builder.config();

        assert_eq!(config.sample_rate, 44100.0);
        assert_eq!(config.max_voices, 8);
        assert_eq!(config.voice_mode, VoiceMode::Poly);
    }

    #[test]
    fn test_builder_chain() {
        let builder = SynthBuilder::new(48000.0)
            .poly(16)
            .oscillator(OscillatorType::Square { pulse_width: 0.5 })
            .filter(FilterType::Moog {
                cutoff: 2000.0,
                resonance: 0.7,
            })
            .envelope(0.01, 0.2, 0.6, 0.3)
            .voice_stealing(AllocationStrategy::Quietest);

        let config = builder.config();
        assert_eq!(config.sample_rate, 48000.0);
        assert_eq!(config.max_voices, 16);
        assert!(matches!(
            config.oscillator,
            OscillatorType::Square { pulse_width: 0.5 }
        ));
        assert!(matches!(
            config.filter,
            FilterType::Moog {
                cutoff: 2000.0,
                resonance: 0.7
            }
        ));
    }

    #[test]
    fn test_mono_mode() {
        let builder = SynthBuilder::new(44100.0).mono();
        assert_eq!(builder.config().max_voices, 1);
        assert_eq!(builder.config().voice_mode, VoiceMode::Mono);
    }

    #[test]
    fn test_legato_mode() {
        let builder = SynthBuilder::new(44100.0).legato();
        assert_eq!(builder.config().max_voices, 1);
        assert_eq!(builder.config().voice_mode, VoiceMode::Legato);
    }

    #[test]
    fn test_envelope_presets() {
        let organ = EnvelopeConfig::organ();
        assert!(organ.attack < 0.01);
        assert_eq!(organ.sustain, 1.0);

        let pluck = EnvelopeConfig::pluck();
        assert!(pluck.attack < 0.01);
        assert_eq!(pluck.sustain, 0.0);

        let pad = EnvelopeConfig::pad();
        assert!(pad.attack > 0.1);
        assert!(pad.release > 0.5);
    }
}
