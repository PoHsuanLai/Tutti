//! Synthesizer building blocks for Tutti.
//!
//! Provides polyphonic synthesis via [`SynthHandle`] (accessed through `engine.synth()`).
//!
//! ```ignore
//! let synth = engine.synth()
//!     .saw()
//!     .poly(8)
//!     .filter_moog(2000.0, 0.7)
//!     .adsr(0.01, 0.2, 0.6, 0.3)
//!     .unison(3, 15.0)
//!     .build()?;
//!
//! let synth_id = engine.graph(|net| net.add(synth).master());
//! engine.note_on(synth_id, Note::C4, 100);
//! ```

pub mod error;
pub use error::{Error, Result};

mod voice;
pub(crate) use voice::{
    AllocationResult, AllocationStrategy, VoiceAllocator, VoiceAllocatorConfig, VoiceMode,
};

mod unison;
pub(crate) use unison::{UnisonConfig, UnisonVoiceParams};
#[cfg(feature = "midi")]
pub(crate) use unison::UnisonEngine;

mod portamento;
pub(crate) use portamento::{Portamento, PortamentoConfig, PortamentoCurve, PortamentoMode};

mod tuning;
pub(crate) use tuning::Tuning;

#[cfg(feature = "soundfont")]
mod soundfont;
#[cfg(feature = "soundfont")]
pub use soundfont::{SoundFontHandle, SoundFontSystem, SoundFontUnit};

mod builder;
#[cfg(feature = "midi")]
pub use builder::PolySynth;
pub(crate) use builder::{EnvelopeConfig, FilterType, OscillatorType, SvfMode, SynthBuilder};

#[cfg(feature = "midi")]
mod handle;
#[cfg(feature = "midi")]
pub use handle::SynthHandle;
