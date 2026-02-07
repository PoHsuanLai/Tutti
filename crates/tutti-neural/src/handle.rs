//! Fluent handle for neural audio operations.

use crate::{Error, NeuralSystem, Result};
use std::sync::Arc;
use tutti_core::ArcNeuralEffectBuilder;
#[cfg(feature = "midi")]
use tutti_core::ArcNeuralSynthBuilder;

/// Fluent handle for neural audio operations.
///
/// Works whether or not neural is enabled. Methods return errors when disabled.
#[derive(Clone)]
pub struct NeuralHandle {
    neural: Option<Arc<NeuralSystem>>,
}

impl NeuralHandle {
    #[doc(hidden)]
    pub fn new(neural: Option<Arc<NeuralSystem>>) -> Self {
        Self { neural }
    }

    #[cfg(feature = "midi")]
    pub fn load_synth(&self, path: &str) -> Result<ArcNeuralSynthBuilder> {
        self.require()?.load_synth_model(path)
    }

    pub fn load_effect(&self, path: &str) -> Result<ArcNeuralEffectBuilder> {
        self.require()?.load_effect_model(path)
    }

    pub fn has_gpu(&self) -> bool {
        self.neural.as_ref().map(|n| n.has_gpu()).unwrap_or(false)
    }

    pub fn sample_rate(&self) -> f32 {
        self.neural.as_ref().map(|n| n.sample_rate()).unwrap_or(0.0)
    }

    pub fn buffer_size(&self) -> usize {
        self.neural.as_ref().map(|n| n.buffer_size()).unwrap_or(0)
    }

    pub fn is_enabled(&self) -> bool {
        self.neural.is_some()
    }

    pub fn inner(&self) -> Option<&Arc<NeuralSystem>> {
        self.neural.as_ref()
    }

    fn require(&self) -> Result<&NeuralSystem> {
        self.neural
            .as_deref()
            .ok_or_else(|| Error::InvalidConfig("Neural subsystem not enabled".to_string()))
    }
}
