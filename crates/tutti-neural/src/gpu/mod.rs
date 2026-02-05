//! GPU-accelerated neural inference internals.
//!
//! Pure tensor API: register models, run forward passes.
//! The engine doesn't know about synths, effects, MIDI, or audio — just tensors.

pub(crate) mod batch;
pub(crate) mod effect_queue;
#[cfg(feature = "midi")]
mod midi_state;
pub(crate) mod queue;

pub use effect_queue::{shared_effect_queue, SharedEffectAudioQueue};
#[cfg(feature = "midi")]
pub use midi_state::{MidiState, MIDI_FEATURE_COUNT};
pub use queue::ControlParams;

// Re-export from tutti-core (canonical definitions)
pub use tutti_core::NeuralModelId;
