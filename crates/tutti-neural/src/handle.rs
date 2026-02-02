//! Fluent handle for neural audio operations

use crate::{EffectHandle, Error, NeuralModel, NeuralSystem, Result, SynthHandle};
use std::sync::Arc;

/// Fluent handle for neural audio operations.
///
/// Works whether or not neural is enabled. Methods return errors
/// when disabled.
///
/// # Example
/// ```ignore
/// // Always works - no Option<> unwrapping needed
/// let neural = engine.neural();
/// neural.load_synth("model.mpk")?;  // Error if disabled
/// neural.run();  // No-op if disabled
/// ```
pub struct NeuralHandle {
    neural: Option<Arc<NeuralSystem>>,
}

impl NeuralHandle {
    /// Create a new handle (internal - use via TuttiEngine)
    #[doc(hidden)]
    pub fn new(neural: Option<Arc<NeuralSystem>>) -> Self {
        Self { neural }
    }

    // Model loading (top-level, commonly used)
    /// Load a neural synth model.
    ///
    /// Returns an error if neural subsystem is not enabled.
    pub fn load_synth(&self, path: &str) -> Result<NeuralModel> {
        self.neural
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("Neural subsystem not enabled".to_string()))?
            .load_synth_model(path)
    }

    /// Load a neural effect model.
    ///
    /// Returns an error if neural subsystem is not enabled.
    pub fn load_effect(&self, path: &str) -> Result<NeuralModel> {
        self.neural
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("Neural subsystem not enabled".to_string()))?
            .load_effect_model(path)
    }

    // Sub-handles
    /// Get the synth sub-handle for synthesis operations.
    ///
    /// When neural is disabled, returns a disabled handle that errors on use.
    pub fn synth(&self) -> Option<SynthHandle> {
        self.neural.as_ref().map(|n| n.synth())
    }

    /// Get the effects sub-handle for effect operations.
    ///
    /// When neural is disabled, returns a disabled handle that errors on use.
    pub fn effects(&self) -> Option<EffectHandle> {
        self.neural.as_ref().map(|n| n.effects())
    }

    // System info
    /// Check if GPU backend is available.
    ///
    /// Returns false when neural is disabled.
    pub fn has_gpu(&self) -> bool {
        self.neural
            .as_ref()
            .map(|n| n.has_gpu())
            .unwrap_or(false)
    }

    /// Get the current sample rate.
    ///
    /// Returns 0.0 when neural is disabled.
    pub fn sample_rate(&self) -> f32 {
        self.neural
            .as_ref()
            .map(|n| n.sample_rate())
            .unwrap_or(0.0)
    }

    /// Get the current buffer size.
    ///
    /// Returns 0 when neural is disabled.
    pub fn buffer_size(&self) -> usize {
        self.neural
            .as_ref()
            .map(|n| n.buffer_size())
            .unwrap_or(0)
    }

    /// Check if neural subsystem is enabled.
    pub fn is_enabled(&self) -> bool {
        self.neural.is_some()
    }

    /// Get reference to inner NeuralSystem (advanced use).
    ///
    /// Returns None when neural is disabled.
    pub fn inner(&self) -> Option<&Arc<NeuralSystem>> {
        self.neural.as_ref()
    }
}

impl Clone for NeuralHandle {
    fn clone(&self) -> Self {
        Self {
            neural: self.neural.clone(),
        }
    }
}
