//! Neural audio synthesis and effects — framework-agnostic orchestration.
//!
//! Pure tensor API: register models, run forward passes.
//! AudioUnit nodes handle the translation to/from synth params and audio buffers.
//!
//! This crate contains NO ML framework dependencies. The inference backend
//! is provided via [`tutti_core::BackendFactory`] at build time. Use
//! `tutti-burn` for a Burn-based backend, or bring your own (ONNX Runtime,
//! candle, etc.).
//!
//! ## Usage
//!
//! ```rust,ignore
//! use tutti_neural::NeuralSystem;
//! use tutti_core::{NeuralSynthBuilder, NeuralEffectBuilder};
//!
//! let neural = NeuralSystem::builder()
//!     .sample_rate(44100.0)
//!     .buffer_size(512)
//!     .backend(my_backend_factory())
//!     .build()?;
//!
//! let synth = neural.load_synth_model("violin.onnx")?;
//! let voice = synth.build_voice()?;  // Box<dyn AudioUnit>
//!
//! let effect = neural.load_effect_model("amp_sim.onnx")?;
//! let fx = effect.build_effect()?;   // Box<dyn AudioUnit>
//! ```

// Error types
mod error;
pub use error::{Error, Result};

// System facade
mod system;
pub use system::{GpuInfo, NeuralSystem, NeuralSystemBuilder};

// Re-export InferenceConfig from tutti-core (canonical definition)
pub use tutti_core::InferenceConfig;

// Fluent handle
mod handle;
pub use handle::NeuralHandle;

// Unified inference engine
mod engine;
pub use engine::{NeuralEngine, ResponseChannel, TensorRequest};

// AudioUnit nodes
#[cfg(feature = "midi")]
mod synth_node;
#[cfg(feature = "midi")]
pub use synth_node::NeuralSynthNode;

mod effect_node;
pub use effect_node::NeuralEffectNode;

// Re-export core traits for convenience
pub use tutti_core::{
    ArcNeuralEffectBuilder, ArcNeuralSynthBuilder, NeuralEffectBuilder, NeuralModelId,
    NeuralSynthBuilder,
};

// Internal
pub(crate) mod gpu;
