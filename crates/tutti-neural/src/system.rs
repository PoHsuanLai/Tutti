//! Neural audio system — unified engine for synthesis and effects.

use crate::effect_node::NeuralEffectNode;
use crate::engine::{NeuralEngine, TensorRequest};
use crate::error::Result;
use crate::gpu::{shared_effect_queue, NeuralModelId};
#[cfg(feature = "midi")]
use crate::synth_node::NeuralSynthNode;

use crossbeam_channel::Sender;
use std::sync::Arc;
pub use tutti_core::InferenceConfig;
use tutti_core::{AudioUnit, BackendFactory, InferenceBackend};

/// Main neural audio system.
#[derive(Clone)]
pub struct NeuralSystem {
    inner: Arc<NeuralSystemInner>,
}

struct NeuralSystemInner {
    has_gpu: bool,
    inference_config: InferenceConfig,
    sample_rate: f32,
    buffer_size: usize,
    engine: NeuralEngine,
}

impl NeuralSystem {
    pub fn builder() -> NeuralSystemBuilder {
        NeuralSystemBuilder::default()
    }

    #[cfg(feature = "midi")]
    pub fn load_synth_model(&self, name: &str) -> Result<Arc<dyn tutti_core::NeuralSynthBuilder>> {
        let model_name = stem_or(name, "Unknown");

        // TODO: Load actual model weights from file
        let id = self.inner.engine.register_model(|backend| {
            backend.register_model(Box::new(|data, _shape| data.to_vec()))
        })?;

        Ok(Arc::new(SynthBuilder {
            model_id: id,
            name: model_name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
            midi_registry: None,
        }))
    }

    pub fn load_effect_model(
        &self,
        name: &str,
    ) -> Result<Arc<dyn tutti_core::NeuralEffectBuilder>> {
        let model_name = stem_or(name, "Unknown");

        // TODO: Load actual model weights from file
        let id = self.inner.engine.register_model(|backend| {
            backend.register_model(Box::new(|data, _shape| data.to_vec()))
        })?;

        Ok(Arc::new(EffectBuilder {
            model_id: id,
            name: model_name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
        }))
    }

    #[cfg(feature = "midi")]
    pub fn register_synth(
        &self,
        name: impl Into<String>,
        f: impl Fn(&[f32]) -> Vec<f32> + Send + 'static,
        midi_registry: Option<tutti_core::midi::MidiRegistry>,
    ) -> Result<Arc<dyn tutti_core::NeuralSynthBuilder>> {
        let name = name.into();
        let id = self
            .inner
            .engine
            .register_model(move |backend: &mut dyn InferenceBackend| {
                backend.register_model(Box::new(move |data, _shape| f(data)))
            })?;

        Ok(Arc::new(SynthBuilder {
            model_id: id,
            name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
            midi_registry,
        }))
    }

    pub fn register_effect(
        &self,
        name: impl Into<String>,
        f: impl Fn(&[f32]) -> Vec<f32> + Send + 'static,
    ) -> Result<Arc<dyn tutti_core::NeuralEffectBuilder>> {
        let name = name.into();
        let id = self
            .inner
            .engine
            .register_model(move |backend: &mut dyn InferenceBackend| {
                backend.register_model(Box::new(move |data, _shape| f(data)))
            })?;

        Ok(Arc::new(EffectBuilder {
            model_id: id,
            name,
            sample_rate: self.inner.sample_rate,
            buffer_size: self.inner.buffer_size,
            request_tx: self.inner.engine.request_sender(),
        }))
    }

    pub fn has_gpu(&self) -> bool {
        self.inner.has_gpu
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

    pub fn update_strategy(&self, strategy: tutti_core::BatchingStrategy) {
        self.inner.engine.update_strategy(strategy);
    }
}

#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub max_memory_mb: Option<u64>,
}

fn stem_or(path: &str, fallback: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(fallback)
        .to_string()
}

pub struct NeuralSystemBuilder {
    inference_config: InferenceConfig,
    sample_rate: f32,
    buffer_size: usize,
    backend_factory: Option<BackendFactory>,
}

impl Default for NeuralSystemBuilder {
    fn default() -> Self {
        Self {
            inference_config: InferenceConfig::default(),
            sample_rate: 44100.0,
            buffer_size: 512,
            backend_factory: None,
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

    pub fn backend(mut self, factory: BackendFactory) -> Self {
        self.backend_factory = Some(factory);
        self
    }

    pub fn build(self) -> Result<NeuralSystem> {
        let backend_factory = self.backend_factory.ok_or_else(|| {
            crate::error::Error::InvalidConfig(
                "No inference backend configured. Use .backend() to set one.".to_string(),
            )
        })?;

        let engine = NeuralEngine::start_with(self.inference_config.clone(), backend_factory)?;
        let has_gpu = false; // Conservative default

        Ok(NeuralSystem {
            inner: Arc::new(NeuralSystemInner {
                has_gpu,
                inference_config: self.inference_config,
                sample_rate: self.sample_rate,
                buffer_size: self.buffer_size,
                engine,
            }),
        })
    }
}

#[cfg(feature = "midi")]
struct SynthBuilder {
    model_id: NeuralModelId,
    name: String,
    sample_rate: f32,
    buffer_size: usize,
    request_tx: Sender<TensorRequest>,
    midi_registry: Option<tutti_core::midi::MidiRegistry>,
}

#[cfg(feature = "midi")]
impl tutti_core::NeuralSynthBuilder for SynthBuilder {
    fn build_voice(&self) -> tutti_core::Result<Box<dyn AudioUnit>> {
        let mut node = NeuralSynthNode::new(
            self.model_id,
            self.sample_rate,
            self.buffer_size,
            self.request_tx.clone(),
        );
        if let Some(ref registry) = self.midi_registry {
            node = node.with_midi_registry(registry.clone());
        }
        Ok(Box::new(node))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> NeuralModelId {
        self.model_id
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use tutti_core::{NeuralEffectBuilder, NeuralSynthBuilder};

    fn test_backend_factory() -> BackendFactory {
        Box::new(|config| {
            Ok(Box::new(TestBackend {
                config,
                models: std::collections::HashMap::new(),
            }) as Box<dyn InferenceBackend>)
        })
    }

    struct TestBackend {
        config: InferenceConfig,
        models: std::collections::HashMap<
            NeuralModelId,
            Box<dyn Fn(&[f32], [usize; 2]) -> Vec<f32> + Send>,
        >,
    }

    impl InferenceBackend for TestBackend {
        fn register_model(
            &mut self,
            f: Box<dyn Fn(&[f32], [usize; 2]) -> Vec<f32> + Send>,
        ) -> NeuralModelId {
            let id = NeuralModelId::new();
            self.models.insert(id, f);
            id
        }

        fn forward_grouped(
            &mut self,
            requests: &[(NeuralModelId, Vec<f32>, usize)],
        ) -> core::result::Result<Vec<Vec<f32>>, tutti_core::InferenceError> {
            Ok(requests
                .iter()
                .map(|(id, data, dim)| {
                    self.models
                        .get(id)
                        .map(|f| {
                            let batch = if *dim > 0 { data.len() / dim } else { 1 };
                            f(data, [batch, *dim])
                        })
                        .unwrap_or_else(|| data.clone())
                })
                .collect())
        }

        fn capabilities(&self) -> tutti_core::BackendCapabilities {
            tutti_core::BackendCapabilities {
                name: "Test".into(),
                supports_batching: false,
                has_gpu: false,
            }
        }

        fn config(&self) -> &InferenceConfig {
            &self.config
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn test_system_creation() {
        assert!(NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .is_ok());
    }

    #[test]
    fn test_builder_defaults() {
        let neural = NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .unwrap();
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
            .backend(test_backend_factory())
            .build()
            .unwrap();

        assert_eq!(neural.sample_rate(), 48000.0);
        assert_eq!(neural.buffer_size(), 256);
        assert_eq!(neural.inference_config().batch_size, 4);
    }

    #[cfg(feature = "midi")]
    #[test]
    fn test_load_synth_model() {
        let neural = NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .unwrap();
        let builder = neural.load_synth_model("test_violin.mpk").unwrap();
        assert_eq!(builder.name(), "test_violin");
    }

    #[test]
    fn test_load_effect_model() {
        let neural = NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .unwrap();
        let builder = neural.load_effect_model("amp_sim.mpk").unwrap();
        assert_eq!(builder.name(), "amp_sim");
    }

    #[cfg(feature = "midi")]
    #[test]
    fn test_synth_builder_trait() {
        let neural = NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .unwrap();
        let builder = neural.load_synth_model("test.mpk").unwrap();
        let voice = builder.build_voice().unwrap();
        assert_eq!(voice.inputs(), 0);
        assert_eq!(voice.outputs(), 2);
    }

    #[test]
    fn test_effect_builder_trait() {
        let neural = NeuralSystem::builder()
            .backend(test_backend_factory())
            .build()
            .unwrap();
        let builder = neural.load_effect_model("test.mpk").unwrap();
        let effect = builder.build_effect().unwrap();
        assert_eq!(effect.inputs(), 2);
        assert_eq!(effect.outputs(), 2);
        assert_eq!(builder.latency(), 512);
    }

    #[test]
    fn test_no_backend_fails() {
        assert!(NeuralSystem::builder().build().is_err());
    }
}
