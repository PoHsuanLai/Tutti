//! Neural audio synthesis and effects.
//!
//! Framework-agnostic orchestration for neural inference. Provide a backend
//! via [`tutti_core::BackendFactory`] (e.g., `tutti-burn`).
//!
//! ```rust,ignore
//! let neural = NeuralSystem::builder()
//!     .backend(my_backend_factory())
//!     .build()?;
//!
//! let synth = neural.load_synth_model("violin.mpk")?;
//! let effect = neural.load_effect_model("amp_sim.mpk")?;
//! ```

mod error;
pub use error::{Error, Result};

mod system;
pub use system::{GpuInfo, NeuralSystem, NeuralSystemBuilder};

pub use tutti_core::InferenceConfig;

mod handle;
pub use handle::NeuralHandle;

pub(crate) mod engine;

#[cfg(feature = "midi")]
mod synth_node;
#[cfg(feature = "midi")]
pub use synth_node::NeuralSynthNode;

mod effect_node;
pub use effect_node::NeuralEffectNode;

pub use tutti_core::{
    ArcNeuralEffectBuilder, ArcNeuralSynthBuilder, NeuralEffectBuilder, NeuralModelId,
    NeuralSynthBuilder,
};

pub(crate) mod gpu;
