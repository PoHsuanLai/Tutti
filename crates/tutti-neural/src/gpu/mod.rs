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

mod engine;
pub mod batch;
mod queue;
mod fusion;
mod midi_state;
mod tutti_adapter;
mod graph_batcher;

pub use engine::{NeuralInferenceEngine, NeuralModelId, InferenceConfig, InferenceRequest, ModelType, VoiceId};
pub use queue::{NeuralParamQueue, ControlParams};
pub use midi_state::MidiState;
#[cfg(test)]
pub(crate) use midi_state::MIDI_FEATURE_COUNT;
pub(crate) use tutti_adapter::NeuralEffectNode;
pub(crate) use graph_batcher::GraphAwareBatcher;
