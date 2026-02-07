//! Synthesizer building blocks for Tutti.
//!
//! Provides synth infrastructure that complements FunDSP's DSP primitives:
//!
//! - **[`SynthBuilder`]** - Fluent API for creating complete synthesizers
//! - **[`VoiceAllocator`]** - Polyphonic voice management with stealing strategies
//! - **[`ModulationMatrix`]** - Route modulation sources to destinations
//! - **[`UnisonConfig`]** - Voice detuning and stereo spread
//! - **[`Portamento`]** - Pitch glide between notes
//! - **[`Tuning`]** - Microtuning and alternative temperaments
//! - **[`SoundFontSynth`]** - SoundFont (.sf2) synthesis (feature: `soundfont`)
//!
//! # Quick Start
//!
//! ```ignore
//! use tutti_synth::{SynthBuilder, OscillatorType, FilterType, UnisonConfig};
//!
//! let mut synth = SynthBuilder::new(44100.0)
//!     .poly(8)
//!     .oscillator(OscillatorType::Saw)
//!     .filter(FilterType::Moog { cutoff: 2000.0, resonance: 0.7 })
//!     .envelope(0.01, 0.2, 0.6, 0.3)
//!     .unison(UnisonConfig::builder().voices(3).detune(15.0).spread(0.5).build())
//!     .build()?;
//! ```
//!
//! # Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `soundfont` | SoundFont (.sf2) synthesis |
//! | `full` | Everything |
//!
//! # Limits
//!
//! Fixed limits for RT-safe, allocation-free operation:
//!
//! | Constant | Value | Description |
//! |----------|-------|-------------|
//! | [`MAX_MOD_ROUTES`] | 32 | Maximum modulation matrix routes |
//! | [`MAX_LFOS`] | 8 | Maximum LFO sources |
//! | [`MAX_ENVELOPES`] | 4 | Maximum envelope sources |
//! | [`MAX_UNISON_VOICES`] | 16 | Maximum unison voices per note |

pub mod error;
pub use error::{Error, Result};

mod voice;

pub use voice::{
    AllocationResult, AllocationStrategy, VoiceAllocator, VoiceAllocatorConfig, VoiceMode,
};

mod modulation;

pub use modulation::{
    ModDestination, ModRoute, ModSource, ModulationMatrix, ModulationMatrixConfig,
    MAX_ENVELOPES, MAX_LFOS, MAX_MOD_ROUTES,
};

// Internal value struct used by builder
#[cfg(feature = "midi")]
pub(crate) use modulation::ModSourceValues;

mod unison;

pub use unison::{UnisonConfig, UnisonConfigBuilder, UnisonVoiceParams, MAX_UNISON_VOICES};

// UnisonEngine is used internally by builder module
#[cfg(feature = "midi")]
pub(crate) use unison::UnisonEngine;

mod portamento;

pub use portamento::{Portamento, PortamentoConfig, PortamentoCurve, PortamentoMode};

mod tuning;

pub use tuning::{ScaleDegree, Tuning, A4_FREQ, A4_NOTE};

#[cfg(feature = "soundfont")]
mod soundfont;

#[cfg(feature = "soundfont")]
pub use soundfont::{SoundFontHandle, SoundFontSynth, SoundFontSystem, SoundFontUnit};

mod builder;

#[cfg(feature = "midi")]
pub use builder::PolySynth;
pub use builder::{EnvelopeConfig, FilterType, OscillatorType, SynthBuilder};

// Filter mode enums are public (needed for FilterType configuration)
pub use builder::{BiquadMode, SvfMode};

#[cfg(feature = "midi")]
mod handle;

#[cfg(feature = "midi")]
pub use handle::SynthHandle;
