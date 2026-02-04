//! GPU-accelerated neural inference internals.
//!
//! Pure tensor API: register models, run forward passes.
//! The engine doesn't know about synths, effects, MIDI, or audio — just tensors.

pub(crate) mod batch;
pub(crate) mod effect_queue;
pub(crate) mod engine;
pub(crate) mod fusion;
mod midi_state;
pub(crate) mod queue;

pub use effect_queue::{shared_effect_queue, SharedEffectAudioQueue};
pub use engine::{InferenceConfig, NeuralModelId};
pub use midi_state::MidiState;
pub use queue::ControlParams;
