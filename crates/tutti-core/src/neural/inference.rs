//! Inference backend abstraction — framework-agnostic neural inference.
//!
//! Defines the [`InferenceBackend`] trait that ML frameworks (Burn, ONNX Runtime,
//! candle, etc.) implement. All operations use flat `&[f32]` data — no
//! framework-specific tensor types cross the boundary.

use super::metadata::NeuralModelId;
use crate::compat::{Box, String, Vec};
use core::any::Any;
use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    pub batch_size: usize,
    pub quantize: bool,
    /// Enable kernel fusion (CubeCL).
    pub enable_fusion: bool,
    /// Use graph-aware batching instead of timing-based.
    pub use_graph_aware_batching: bool,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            batch_size: 8,
            quantize: false,
            enable_fusion: true,
            use_graph_aware_batching: false,
        }
    }
}

#[derive(Debug)]
pub enum InferenceError {
    ModelNotFound(NeuralModelId),
    ForwardFailed(String),
    BackendInit(String),
}

impl fmt::Display for InferenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModelNotFound(id) => write!(f, "Model not found: {}", id),
            Self::ForwardFailed(msg) => write!(f, "Forward pass failed: {}", msg),
            Self::BackendInit(msg) => write!(f, "Backend initialization failed: {}", msg),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackendCapabilities {
    /// e.g. "Burn/NdArray", "ONNX Runtime"
    pub name: String,
    pub supports_batching: bool,
    pub has_gpu: bool,
}

pub type ForwardFn = Box<dyn Fn(&[f32], [usize; 2]) -> Vec<f32> + Send>;

/// Abstraction over ML inference backends (Burn, ONNX Runtime, candle, etc.)
///
/// Operates on flat `&[f32]` data — no framework-specific tensor types in the API.
/// Implementations handle tensor conversion internally.
///
/// # Lifecycle
///
/// 1. Create via [`BackendFactory`]
/// 2. Register models with [`register_model`](Self::register_model)
/// 3. Run inference with [`forward_grouped`](Self::forward_grouped)
///
/// # Thread Safety
///
/// The backend lives on a single dedicated inference thread. It is created
/// there via [`BackendFactory`] and never sent to another thread. No `Send`
/// or `Sync` bound is required — the factory closure is `Send`, but the
/// backend it produces stays on the inference thread.
pub trait InferenceBackend {
    /// Register a model from a forward closure.
    ///
    /// The closure receives flat `&[f32]` data and `[batch, features]` shape,
    /// and returns flat `Vec<f32>` output. The backend handles any tensor
    /// conversion internally.
    ///
    /// The closure is `Send` (it crosses the thread boundary to the inference
    /// thread) but not required to be `Sync`.
    fn register_model(&mut self, f: ForwardFn) -> NeuralModelId;

    /// Run grouped forward pass.
    ///
    /// Each request is `(model_id, flat_data, feature_dim)`.
    /// Returns results in the same order as input requests.
    /// The backend may batch requests with the same `model_id` and `feature_dim`.
    fn forward_grouped(
        &mut self,
        requests: &[(NeuralModelId, Vec<f32>, usize)],
    ) -> core::result::Result<Vec<Vec<f32>>, InferenceError>;

    fn capabilities(&self) -> BackendCapabilities;

    fn config(&self) -> &InferenceConfig;

    /// Downcast to concrete type for native model registration.
    ///
    /// This allows framework-specific code (e.g. tutti-burn) to register
    /// native models without going through the closure API.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Factory function to create an [`InferenceBackend`].
///
/// Called once on the inference thread to initialize the backend.
pub type BackendFactory = Box<
    dyn FnOnce(InferenceConfig) -> core::result::Result<Box<dyn InferenceBackend>, InferenceError>
        + Send,
>;
