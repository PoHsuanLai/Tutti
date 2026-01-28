//! GPU-accelerated neural audio synthesis and effects.
//!
//! ```rust,ignore
//! use tutti_neural::NeuralSystem;
//!
//! let neural = NeuralSystem::new()
//!     .sample_rate(44100.0)
//!     .buffer_size(512)
//!     .build()?;
//!
//! let model = neural.load_synth_model("violin.onnx")?;
//! let voice = neural.synth().build_voice(&model)?;
//! ```

pub mod error;
pub use error::{Error, Result};

mod system;
pub use system::{
    EffectHandle, GpuInfo, NeuralModel, NeuralSystem, NeuralSystemBuilder, SynthHandle,
};

pub use gpu::{InferenceConfig, ModelType, VoiceId};
pub use tutti_core::neural::{BatchingStrategy, NeuralNodeManager};
pub use tutti_core::AudioUnit;

pub mod model;

pub(crate) mod backend;
pub(crate) mod effects;
pub(crate) mod gpu;
pub(crate) mod synthesis;
