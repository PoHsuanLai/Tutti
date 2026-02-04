//! Synth building blocks for Tutti.
//!
//! This crate provides synthesizer infrastructure that complements FunDSP:
//! - Voice allocation and management
//! - Modulation matrix
//! - Unison engine
//! - Portamento (pitch glide)
//! - Microtuning
//! - SoundFont synthesis (via RustySynth)
//! - **SynthBuilder** - Fluent API for creating complete synthesizers
//!
//! FunDSP provides the DSP primitives (oscillators, filters, envelopes).
//! This crate provides the synth-specific infrastructure to build complete instruments.
//!
//! For MIDI processing, use `tutti-midi-io` directly - it provides all the
//! building blocks needed (MidiEvent, ChannelVoiceMsg, MpeProcessor, etc.).
//!
//! ## Feature Flags
//!
//! - `voice-alloc` - Voice allocator with stealing strategies
//! - `modulation` - Modulation matrix (sources → destinations)
//! - `unison` - Unison engine (detune + stereo spread)
//! - `portamento` - Pitch glide
//! - `tuning` - Microtuning support
//! - `soundfont` - SoundFont (.sf2) synthesis
//! - `synth-blocks` - All building blocks (voice-alloc, modulation, unison, portamento)
//! - `builder` - SynthBuilder fluent API (requires synth-blocks + tuning)
//! - `full` - Everything
//!
//! ## Example
//!
//! ```ignore
//! use tutti_synth::{VoiceAllocator, VoiceAllocatorConfig, AllocationStrategy};
//! use tutti_synth::{Portamento, PortamentoConfig};
//! use tutti_synth::Tuning;
//! use tutti_midi_io::{MidiEvent, ChannelVoiceMsg};
//!
//! // Set up voice allocator
//! let config = VoiceAllocatorConfig {
//!     max_voices: 8,
//!     strategy: AllocationStrategy::Oldest,
//!     ..Default::default()
//! };
//! let mut allocator = VoiceAllocator::new(config);
//!
//! // Set up portamento
//! let mut portamento = Portamento::new(PortamentoConfig::default(), 44100.0);
//!
//! // Set up tuning
//! let tuning = Tuning::equal_temperament();
//!
//! // Process MIDI directly
//! match event.msg {
//!     ChannelVoiceMsg::NoteOn { note, velocity } => {
//!         let vel_norm = velocity as f32 / 127.0;
//!         let result = allocator.allocate(note, channel, vel_norm);
//!         let freq = tuning.note_to_freq(note);
//!         portamento.set_target(freq, result.is_legato);
//!     }
//!     ChannelVoiceMsg::NoteOff { note, .. } => {
//!         allocator.release(note, channel);
//!     }
//!     ChannelVoiceMsg::ControlChange { control } => {
//!         if let ControlChange::CC { control: 64, value } = control {
//!             allocator.sustain_pedal(channel, value >= 64);
//!         }
//!     }
//!     _ => {}
//! }
//!
//! // In audio loop
//! let current_freq = portamento.tick();
//! // Use current_freq with FunDSP oscillator
//! ```

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
pub use builder::{
    EnvelopeConfig, FilterType, OscillatorType, PolySynth, SynthBuilder,
};

// Filter mode enums are public (needed for FilterType configuration)
#[cfg(feature = "builder")]
pub use builder::{BiquadMode, SvfMode};
