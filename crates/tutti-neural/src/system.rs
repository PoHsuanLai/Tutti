//! Neural audio system — unified engine for synthesis and effects.

use crate::backend::BackendPool;
use crate::effect_node::NeuralEffectNode;
use crate::engine::{NeuralEngine, TensorRequest};
use crate::error::Result;
use crate::gpu::{shared_effect_queue, NeuralModelId};
use crate::synth_node::NeuralSynthNode;

pub use crate::gpu::InferenceConfig;
use burn::backend::NdArray;
use crossbeam_channel::Sender;
use std::sync::Arc;
use tutti_core::AudioUnit;

// ============================================================================
// NeuralSystem
// ============================================================================

/// Main neural audio system.
///
/// Wraps a single `NeuralEngine` (one inference thread) and provides
/// ergonomic methods for loading models and building AudioUnit instances.
#[derive(Clone)]
pub struct NeuralSystem {
    inner: Arc<NeuralSystemInner>,
}

struct NeuralSystemInner {
    backend_pool: BackendPool,
    inference_config: InferenceConfig,
    sample_rate: f32,
    buffer_size: usize,
    engine: NeuralEngine,
}

impl NeuralSystem {
    pub fn builder() -> NeuralSystemBuilder {
        NeuralSystemBuilder::default()
    }

    /// Load a neural synth model.
    ///
    /// Returns an `Arc<dyn NeuralSynthBuilder>` that can build voices
    /// and integrates with tutti-core's graph-aware batching.
    pub fn load_synth_model(&self, name: &str) -> Result<Arc<dyn tutti_core::NeuralSynthBuilder>> {
        let model_name = stem_or(name, "Unknown");

        // TODO: Load actual model weights from file.
        let id = self.inner.engine.register_model(|| {
            crate::gpu::fusion::NeuralModel::<NdArray>::from_forward(|input| input)
        })?;

        Ok(Arc::new(SynthBuilder {
            model_id: id,
            name: model_name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
        }))
    }

    /// Load a neural effect model.
    ///
    /// Returns an `Arc<dyn NeuralEffectBuilder>` that can build effects
    /// and integrates with tutti-core's graph-aware batching.
    pub fn load_effect_model(
        &self,
        name: &str,
    ) -> Result<Arc<dyn tutti_core::NeuralEffectBuilder>> {
        let model_name = stem_or(name, "Unknown");

        // TODO: Load actual model weights from file.
        let id = self.inner.engine.register_model(|| {
            crate::gpu::fusion::NeuralModel::<NdArray>::from_forward(|input| input)
        })?;

        Ok(Arc::new(EffectBuilder {
            model_id: id,
            name: model_name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
        }))
    }

    // ==================== System Info ====================

    pub fn has_gpu(&self) -> bool {
        self.inner.backend_pool.has_gpu()
    }

    pub fn gpu_info(&self) -> Option<GpuInfo> {
        self.inner.backend_pool.gpu_info().map(|info| GpuInfo {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            max_memory_mb: info.max_memory_mb,
        })
    }

    pub fn sample_rate(&self) -> f32 {
        self.inner.sample_rate
    }

    pub fn buffer_size(&self) -> usize {
        self.inner.buffer_size
    }

    pub fn inference_config(&self) -> &InferenceConfig {
        &self.inner.inference_config
    }

    /// Forward a batching strategy to the inference engine.
    ///
    /// Best-effort: silently drops if the command channel is full.
    pub fn update_strategy(&self, strategy: tutti_core::BatchingStrategy) {
        self.inner.engine.update_strategy(strategy);
    }
}

/// GPU device information.
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub max_memory_mb: Option<u64>,
}

/// Extract file stem, or fallback.
fn stem_or(path: &str, fallback: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(fallback)
        .to_string()
}

// ============================================================================
// NeuralSystemBuilder
// ============================================================================

pub struct NeuralSystemBuilder {
    inference_config: InferenceConfig,
    sample_rate: f32,
    buffer_size: usize,
}

impl Default for NeuralSystemBuilder {
    fn default() -> Self {
        Self {
            inference_config: InferenceConfig::default(),
            sample_rate: 44100.0,
            buffer_size: 512,
        }
    }
}

impl NeuralSystemBuilder {
    pub fn inference_config(mut self, config: InferenceConfig) -> Self {
        self.inference_config = config;
        self
    }

    pub fn sample_rate(mut self, sample_rate: f32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    pub fn buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = buffer_size;
        self
    }

    pub fn build(self) -> Result<NeuralSystem> {
        let backend_pool = BackendPool::new()?;
        let engine = NeuralEngine::start(self.inference_config.clone())?;

        Ok(NeuralSystem {
            inner: Arc::new(NeuralSystemInner {
                backend_pool,
                inference_config: self.inference_config,
                sample_rate: self.sample_rate,
                buffer_size: self.buffer_size,
                engine,
            }),
        })
    }
}

// ============================================================================
// Builder impls (tutti-core trait integration)
// ============================================================================

/// Implements `tutti_core::NeuralSynthBuilder` for a loaded synth model.
struct SynthBuilder {
    model_id: NeuralModelId,
    name: String,
    sample_rate: f32,
    buffer_size: usize,
    request_tx: Sender<TensorRequest>,
}

impl tutti_core::NeuralSynthBuilder for SynthBuilder {
    fn build_voice(&self) -> tutti_core::Result<Box<dyn AudioUnit>> {
        let node = NeuralSynthNode::new(
            self.model_id,
            self.sample_rate,
            self.buffer_size,
            self.request_tx.clone(),
        );
        Ok(Box::new(node))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> NeuralModelId {
        self.model_id
    }
}

/// Implements `tutti_core::NeuralEffectBuilder` for a loaded effect model.
struct EffectBuilder {
    model_id: NeuralModelId,
    name: String,
    sample_rate: f32,
    buffer_size: usize,
    request_tx: Sender<TensorRequest>,
}

impl tutti_core::NeuralEffectBuilder for EffectBuilder {
    fn build_effect(&self) -> tutti_core::Result<Box<dyn AudioUnit>> {
        let queue = shared_effect_queue(2, self.buffer_size);
        let node = NeuralEffectNode::new(
            self.model_id,
            self.buffer_size,
            queue,
            self.request_tx.clone(),
        )
        .with_sample_rate(self.sample_rate);
        Ok(Box::new(node))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> NeuralModelId {
        self.model_id
    }

    fn latency(&self) -> usize {
        self.buffer_size
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    // Traits in scope for calling build_voice(), build_effect(), etc. on Arc<dyn Trait>
    #[allow(unused_imports)]
    use tutti_core::{NeuralEffectBuilder, NeuralSynthBuilder};

    #[test]
    fn test_neural_system_creation() {
        let neural = NeuralSystem::builder().build();
        assert!(neural.is_ok());
    }

    #[test]
    fn test_builder_defaults() {
        let neural = NeuralSystem::builder().build().unwrap();
        assert_eq!(neural.sample_rate(), 44100.0);
        assert_eq!(neural.buffer_size(), 512);
    }

    #[test]
    fn test_builder_custom_config() {
        let neural = NeuralSystem::builder()
            .sample_rate(48000.0)
            .buffer_size(256)
            .inference_config(InferenceConfig {
                batch_size: 4,
                ..InferenceConfig::default()
            })
            .build()
            .unwrap();

        assert_eq!(neural.sample_rate(), 48000.0);
        assert_eq!(neural.buffer_size(), 256);
        assert_eq!(neural.inference_config().batch_size, 4);
    }

    #[test]
    fn test_clone() {
        let neural = NeuralSystem::builder().build().unwrap();
        let neural2 = neural.clone();
        assert_eq!(neural.has_gpu(), neural2.has_gpu());
    }

    #[test]
    fn test_load_synth_model() {
        let neural = NeuralSystem::builder().build().unwrap();
        let builder = neural.load_synth_model("test_violin.mpk").unwrap();
        assert_eq!(builder.name(), "test_violin");
    }

    #[test]
    fn test_load_effect_model() {
        let neural = NeuralSystem::builder().build().unwrap();
        let builder = neural.load_effect_model("amp_sim.mpk").unwrap();
        assert_eq!(builder.name(), "amp_sim");
    }

    #[test]
    fn test_synth_builder_trait() {
        let neural = NeuralSystem::builder().build().unwrap();
        let builder = neural.load_synth_model("test.mpk").unwrap();

        // Uses tutti_core::NeuralSynthBuilder trait
        let voice = builder.build_voice();
        assert!(voice.is_ok());

        let voice = voice.unwrap();
        assert_eq!(voice.inputs(), 0);
        assert_eq!(voice.outputs(), 2);
    }

    #[test]
    fn test_effect_builder_trait() {
        let neural = NeuralSystem::builder().build().unwrap();
        let builder = neural.load_effect_model("test.mpk").unwrap();

        // Uses tutti_core::NeuralEffectBuilder trait
        let effect = builder.build_effect();
        assert!(effect.is_ok());

        let effect = effect.unwrap();
        assert_eq!(effect.inputs(), 2);
        assert_eq!(effect.outputs(), 2);
    }

    #[test]
    fn test_synth_builder_model_id() {
        let neural = NeuralSystem::builder().build().unwrap();
        let b1 = neural.load_synth_model("a.mpk").unwrap();
        let b2 = neural.load_synth_model("b.mpk").unwrap();

        // Different models get different IDs
        assert_ne!(b1.model_id().as_u64(), b2.model_id().as_u64());

        // Same builder always returns the same model_id (for batching)
        assert_eq!(b1.model_id(), b1.model_id());
    }

    #[test]
    fn test_effect_builder_latency() {
        let neural = NeuralSystem::builder().buffer_size(256).build().unwrap();
        let builder = neural.load_effect_model("test.mpk").unwrap();
        assert_eq!(builder.latency(), 256);
    }
}
