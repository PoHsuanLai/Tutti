//! GPU-accelerated neural inference engine.
//!
//! Provides GPU inference for neural audio using the Burn ML framework.
//! Includes batch processing, kernel fusion, and lock-free parameter queues
//! for real-time audio processing.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tutti_neural::gpu::{NeuralInferenceEngine, NeuralSynthNode};
//! use tutti_core::AudioUnit;
//!
//! let engine = NeuralInferenceEngine::new(device, config)?;
//! let node = NeuralSynthNode::new(Arc::new(engine), model_id);
//!
//! // Node implements AudioUnit and can be added to Net
//! net.push(Box::new(node));
//! ```

pub mod batch;
mod engine;
mod fusion;
mod graph_batcher;
mod midi_state;
mod queue;
mod tutti_adapter;

pub use engine::{
    InferenceConfig, InferenceRequest, ModelType, NeuralInferenceEngine, NeuralModelId, VoiceId,
};
pub(crate) use graph_batcher::GraphAwareBatcher;
pub use midi_state::MidiState;
#[cfg(test)]
pub(crate) use midi_state::MIDI_FEATURE_COUNT;
pub use queue::{ControlParams, NeuralParamQueue};
pub(crate) use tutti_adapter::NeuralEffectNode;
