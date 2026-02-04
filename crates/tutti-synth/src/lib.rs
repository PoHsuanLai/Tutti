//! Synthesizer building blocks for Tutti.
//!
//! Provides synth infrastructure that complements FunDSP's DSP primitives:
//!
//! - **[`SynthBuilder`]** - Fluent API for creating complete synthesizers
//! - **[`VoiceAllocator`]** - Polyphonic voice management with stealing strategies
//! - **[`ModulationMatrix`]** - Route modulation sources to destinations
//! - **[`UnisonEngine`][UnisonConfig]** - Voice detuning and stereo spread
//! - **[`Portamento`]** - Pitch glide between notes
//! - **[`Tuning`]** - Microtuning and alternative temperaments
//! - **SoundFontSynth** - SoundFont (.sf2) synthesis (feature: `soundfont`)
//!
//! # Quick Start
//!
//! ```ignore
//! use tutti_synth::{SynthBuilder, OscillatorType, FilterType};
//!
//! let mut synth = SynthBuilder::new(44100.0)
//!     .poly(8)
//!     .oscillator(OscillatorType::Saw)
//!     .filter(FilterType::Moog { cutoff: 2000.0, resonance: 0.7 })
//!     .envelope(0.01, 0.2, 0.6, 0.3)
//!     .build()?;
//! ```
//!
//! # Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `voice-alloc` | Voice allocator with stealing strategies |
//! | `modulation` | Modulation matrix |
//! | `unison` | Unison engine (detune + stereo spread) |
//! | `portamento` | Pitch glide |
//! | `tuning` | Microtuning support |
//! | `soundfont` | SoundFont synthesis |
//! | `synth-blocks` | All building blocks |
//! | `builder` | SynthBuilder API (includes synth-blocks + tuning) |
//! | `full` | Everything |

pub mod error;
pub use error::{Error, Result};

// =============================================================================
// Voice Management
// =============================================================================

#[cfg(feature = "voice-alloc")]
mod voice;

#[cfg(feature = "voice-alloc")]
pub use voice::{
    AllocationResult, AllocationStrategy, VoiceAllocator, VoiceAllocatorConfig, VoiceMode,
};

// =============================================================================
// Modulation
// =============================================================================

#[cfg(feature = "modulation")]
mod modulation;

#[cfg(feature = "modulation")]
pub use modulation::{
    ModDestination, ModRoute, ModSource, ModulationMatrix, ModulationMatrixConfig,
};

// Internal value struct used by builder
#[cfg(feature = "modulation")]
pub(crate) use modulation::ModSourceValues;

// =============================================================================
// Unison
// =============================================================================

#[cfg(feature = "unison")]
mod unison;

// UnisonConfig is public (needed for SynthBuilder)
// UnisonEngine, UnisonVoiceParams, MAX_UNISON_VOICES are pub(crate)
#[cfg(feature = "unison")]
pub use unison::UnisonConfig;

// UnisonEngine is used internally by builder module
#[cfg(feature = "unison")]
pub(crate) use unison::UnisonEngine;

// =============================================================================
// Portamento
// =============================================================================

#[cfg(feature = "portamento")]
mod portamento;

#[cfg(feature = "portamento")]
pub use portamento::{Portamento, PortamentoConfig, PortamentoCurve, PortamentoMode};

// =============================================================================
// Microtuning
// =============================================================================

#[cfg(feature = "tuning")]
mod tuning;

#[cfg(feature = "tuning")]
pub use tuning::{Tuning, A4_FREQ, A4_NOTE};

// =============================================================================
// SoundFont Synthesis
// =============================================================================

#[cfg(feature = "soundfont")]
mod soundfont;

#[cfg(feature = "soundfont")]
pub use soundfont::{SoundFontHandle, SoundFontSynth, SoundFontSystem, SoundFontUnit};

// =============================================================================
// SynthBuilder (Fluent API)
// =============================================================================

#[cfg(feature = "builder")]
mod builder;

#[cfg(feature = "builder")]
pub use builder::{EnvelopeConfig, FilterType, OscillatorType, PolySynth, SynthBuilder};

// Filter mode enums are public (needed for FilterType configuration)
#[cfg(feature = "builder")]
pub use builder::{BiquadMode, SvfMode};
