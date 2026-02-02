//! Builder traits for neural synthesis and effects.

use super::metadata::NeuralModelId;
use crate::AudioUnit;
use crate::Result;
use crate::compat::{Arc, Box};

/// Builder for neural synthesis voices.
pub trait NeuralSynthBuilder: Send + Sync {
    /// Build a new voice instance.
    fn build_voice(&self) -> Result<Box<dyn AudioUnit>>;

    /// Get the synth name.
    fn name(&self) -> &str;

    /// Model identifier for batching. Same ID = same GPU batch.
    /// Default: unique ID per instance (no batching).
    fn model_id(&self) -> NeuralModelId {
        NeuralModelId::new()
    }
}

/// Builder for neural audio effects (amp sims, compressors, reverbs).
pub trait NeuralEffectBuilder: Send + Sync {
    /// Build a new effect instance.
    fn build_effect(&self) -> Result<Box<dyn AudioUnit>>;

    /// Get the effect name.
    fn name(&self) -> &str;

    /// Model identifier for batching. Same ID = same GPU batch.
    /// Default: unique ID per instance (no batching).
    fn model_id(&self) -> NeuralModelId {
        NeuralModelId::new()
    }

    /// Latency in samples (for PDC).
    fn latency(&self) -> usize {
        0
    }
}

/// Type alias for Arc-wrapped neural synth builder
pub type ArcNeuralSynthBuilder = Arc<dyn NeuralSynthBuilder>;

/// Type alias for Arc-wrapped neural effect builder
pub type ArcNeuralEffectBuilder = Arc<dyn NeuralEffectBuilder>;
