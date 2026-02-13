//! Builder traits for neural synthesis and effects.

use super::metadata::NeuralModelId;
use crate::compat::{Arc, Box};
use crate::AudioUnit;
use crate::Result;

pub trait NeuralSynthBuilder: Send + Sync {
    fn build_voice(&self) -> Result<Box<dyn AudioUnit>>;

    fn name(&self) -> &str;

    /// Model identifier for batching. Same ID = same GPU batch.
    /// Default: unique ID per instance (no batching).
    fn model_id(&self) -> NeuralModelId {
        NeuralModelId::new()
    }
}

pub trait NeuralEffectBuilder: Send + Sync {
    fn build_effect(&self) -> Result<Box<dyn AudioUnit>>;

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

pub type ArcNeuralSynthBuilder = Arc<dyn NeuralSynthBuilder>;

pub type ArcNeuralEffectBuilder = Arc<dyn NeuralEffectBuilder>;
