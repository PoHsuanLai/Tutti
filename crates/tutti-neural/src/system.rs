//! Neural audio system with synthesis and effects.

use crate::backend::BackendPool;
use crate::effects::SyncEffectBuilder;
use crate::error::{Error, Result};
use crate::gpu::{InferenceConfig, ModelType, NeuralModelId, VoiceId};
use crate::synthesis::SyncNeuralSynthBuilder;
use burn::backend::NdArray;
use std::sync::Arc;
use tutti_core::neural::{BatchingStrategy, NeuralNodeManager};
use tutti_core::AudioUnit;

/// GPU device information.
#[derive(Debug, Clone)]
pub struct GpuInfo {
    pub name: String,
    pub backend: String,
    pub max_memory_mb: Option<u64>,
}

/// Reference to a loaded neural model.
#[derive(Debug, Clone)]
pub struct NeuralModel {
    id: NeuralModelId,
    name: String,
    model_type: ModelType,
}

impl NeuralModel {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn model_type(&self) -> ModelType {
        self.model_type
    }
}

/// Main neural audio system.
#[derive(Clone)]
pub struct NeuralSystem {
    inner: Arc<NeuralSystemInner>,
}

struct NeuralSystemInner {
    backend_pool: BackendPool,

    /// Inference configuration
    inference_config: InferenceConfig,

    /// Sample rate
    sample_rate: f32,

    /// Audio buffer size
    buffer_size: usize,

    /// Loaded synth builders (keyed by model path for dedup)
    synth_builders: dashmap::DashMap<NeuralModelId, Arc<SyncNeuralSynthBuilder>>,

    /// Loaded effect builders (keyed by model path for dedup)
    effect_builders: dashmap::DashMap<NeuralModelId, Arc<SyncEffectBuilder>>,

    /// Neural node registry (shared with audio graph for graph-aware batching)
    neural_node_manager: Arc<NeuralNodeManager>,

    /// Per-thread strategy senders — each inference thread gets its own channel.
    /// `update_batching_strategy()` sends to ALL of them (broadcast semantics).
    strategy_senders: std::sync::Mutex<Vec<crossbeam_channel::Sender<BatchingStrategy>>>,
}

impl NeuralSystem {
    /// Create a builder for configuring the neural system.
    pub fn builder() -> NeuralSystemBuilder {
        NeuralSystemBuilder::default()
    }

    /// Load a neural synth model.
    pub fn load_synth_model(&self, path: &str) -> Result<NeuralModel> {
        let sample_rate = self.inner.sample_rate;
        let buffer_size = self.inner.buffer_size;
        let inference_config = self.inner.inference_config.clone();

        // Create a dedicated channel for this inference thread's strategy updates.
        // Each thread gets its own receiver so all threads receive every update.
        let strategy_rx = if inference_config.use_graph_aware_batching {
            let (tx, rx) = crossbeam_channel::bounded(4);
            self.inner
                .strategy_senders
                .lock()
                .expect("strategy_senders mutex poisoned (previous thread panicked)")
                .push(tx);
            Some(rx)
        } else {
            None
        };

        let builder = SyncNeuralSynthBuilder::new_with_strategy::<NdArray, _>(
            move || {
                let pool = BackendPool::new()?;
                let device = pool.cpu_device().clone();
                let engine =
                    crate::gpu::NeuralInferenceEngine::<NdArray>::new(device, inference_config)?;
                #[allow(clippy::arc_with_non_send_sync)] // NdArray is single-threaded by design
                Ok(Arc::new(engine))
            },
            path,
            sample_rate,
            buffer_size,
            strategy_rx,
        )?;

        let model = NeuralModel {
            id: builder.model_id(),
            name: builder.name().to_string(),
            model_type: ModelType::NeuralSynth,
        };

        self.inner
            .synth_builders
            .insert(model.id, Arc::new(builder));

        Ok(model)
    }

    /// Load a neural effect model.
    pub fn load_effect_model(&self, path: &str) -> Result<NeuralModel> {
        let buffer_size = self.inner.buffer_size;
        let sample_rate = self.inner.sample_rate;
        let inference_config = self.inner.inference_config.clone();

        let builder = SyncEffectBuilder::new::<NdArray, _>(
            move || {
                let pool = BackendPool::new()?;
                let device = pool.cpu_device().clone();
                let engine =
                    crate::gpu::NeuralInferenceEngine::<NdArray>::new(device, inference_config)?;
                #[allow(clippy::arc_with_non_send_sync)] // NdArray is single-threaded by design
                Ok(Arc::new(engine))
            },
            path,
            buffer_size,
            sample_rate,
        )?;

        let model = NeuralModel {
            id: builder.model_id(),
            name: builder.name().to_string(),
            model_type: ModelType::Effect,
        };

        self.inner
            .effect_builders
            .insert(model.id, Arc::new(builder));

        Ok(model)
    }

    // ==================== Sub-Handles ====================

    /// Get the synth sub-handle for synthesis operations.
    pub fn synth(&self) -> SynthHandle {
        SynthHandle {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Get the effects sub-handle for effect operations.
    pub fn effects(&self) -> EffectHandle {
        EffectHandle {
            inner: Arc::clone(&self.inner),
        }
    }

    // ==================== System Info ====================

    /// Check if GPU backend is available.
    pub fn has_gpu(&self) -> bool {
        self.inner.backend_pool.has_gpu()
    }

    /// Get GPU information (if available).
    ///
    /// Returns an owned [`GpuInfo`] snapshot.
    pub fn gpu_info(&self) -> Option<GpuInfo> {
        self.inner.backend_pool.gpu_info().map(|info| GpuInfo {
            name: info.name.clone(),
            backend: format!("{:?}", info.backend),
            max_memory_mb: info.max_memory_mb,
        })
    }

    /// Get the current sample rate.
    pub fn sample_rate(&self) -> f32 {
        self.inner.sample_rate
    }

    /// Get the current buffer size.
    pub fn buffer_size(&self) -> usize {
        self.inner.buffer_size
    }

    /// Get the inference configuration.
    pub fn inference_config(&self) -> &InferenceConfig {
        &self.inner.inference_config
    }

    // ==================== Graph-Aware Batching ====================

    /// Update the batching strategy for all inference threads.
    ///
    /// Call this when the audio graph changes (nodes added/removed/reconnected).
    /// The strategy is computed by `tutti_core::neural::GraphAnalyzer` and pushed
    /// to all inference threads so they can batch requests optimally.
    ///
    /// No-op if `use_graph_aware_batching` is `false` in config.
    pub fn update_batching_strategy(&self, strategy: BatchingStrategy) {
        if !self.inner.inference_config.use_graph_aware_batching {
            return;
        }

        let mut senders = self
            .inner
            .strategy_senders
            .lock()
            .expect("strategy_senders mutex poisoned (previous thread panicked)");
        // Remove disconnected senders (inference thread exited) while broadcasting
        senders.retain(|tx| tx.try_send(strategy.clone()).is_ok());
    }

    /// Get the shared neural node manager.
    ///
    /// Used by the audio graph (TuttiNet) to register/unregister neural nodes.
    /// The same manager is shared with `GraphAnalyzer` for strategy computation.
    pub fn neural_node_manager(&self) -> &Arc<NeuralNodeManager> {
        &self.inner.neural_node_manager
    }
}

// ============================================================================
// NeuralSystemBuilder
// ============================================================================

/// Builder for configuring [`NeuralSystem`].
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

        Ok(NeuralSystem {
            inner: Arc::new(NeuralSystemInner {
                backend_pool,
                inference_config: self.inference_config,
                sample_rate: self.sample_rate,
                buffer_size: self.buffer_size,
                synth_builders: dashmap::DashMap::new(),
                effect_builders: dashmap::DashMap::new(),
                neural_node_manager: Arc::new(NeuralNodeManager::new()),
                strategy_senders: std::sync::Mutex::new(Vec::new()),
            }),
        })
    }
}

// ============================================================================
// Sub-Handles
// ============================================================================

/// Handle for neural synthesis operations.
///
/// Obtained via [`NeuralSystem::synth()`]. Provides methods for building
/// synth voices and sending features for inference.
pub struct SynthHandle {
    inner: Arc<NeuralSystemInner>,
}

impl SynthHandle {
    /// Build a new synth voice from a loaded model.
    ///
    /// Returns a `Box<dyn AudioUnit>` that can be added to the audio graph.
    /// Each call creates a new independent voice instance.
    pub fn build_voice(&self, model: &NeuralModel) -> Result<Box<dyn AudioUnit>> {
        let builder = self.inner.synth_builders.get(&model.id).ok_or_else(|| {
            Error::ModelNotFound(format!("Synth model '{}' not loaded", model.name))
        })?;
        builder.build_voice_sync()
    }

    /// Send pre-computed features for neural inference (RT-safe).
    ///
    /// The caller maintains a `MidiState` per voice, applies MIDI events,
    /// and calls `to_features()` to produce the feature vector.
    ///
    /// Non-blocking — drops the request if the queue is full.
    ///
    /// Returns `true` if queued successfully.
    pub fn send_features(
        &self,
        model: &NeuralModel,
        voice_id: VoiceId,
        features: Vec<f32>,
    ) -> bool {
        if let Some(builder) = self.inner.synth_builders.get(&model.id) {
            builder.send_features_rt(voice_id, features)
        } else {
            false
        }
    }
}

/// Handle for neural effect operations.
///
/// Obtained via [`NeuralSystem::effects()`]. Provides methods for building
/// effect instances and querying latency.
pub struct EffectHandle {
    inner: Arc<NeuralSystemInner>,
}

impl EffectHandle {
    /// Build a new effect instance from a loaded model.
    ///
    /// Returns a `Box<dyn AudioUnit>` that can be added to the audio graph.
    /// Each call creates a new independent effect instance.
    pub fn build_effect(&self, model: &NeuralModel) -> Result<Box<dyn AudioUnit>> {
        let builder = self.inner.effect_builders.get(&model.id).ok_or_else(|| {
            Error::ModelNotFound(format!("Effect model '{}' not loaded", model.name))
        })?;
        builder.build_effect_sync()
    }

    /// Get the processing latency in samples for a loaded model.
    pub fn latency(&self, model: &NeuralModel) -> Option<usize> {
        self.inner
            .effect_builders
            .get(&model.id)
            .map(|b| b.latency())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

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

        // Both share the same backend pool
        assert_eq!(neural.has_gpu(), neural2.has_gpu());
    }

    #[test]
    fn test_sub_handles() {
        let neural = NeuralSystem::builder().build().unwrap();

        let _synth = neural.synth();
        let _effects = neural.effects();
    }

    #[test]
    fn test_neural_model() {
        let model = NeuralModel {
            id: NeuralModelId::new(),
            name: "test_model".to_string(),
            model_type: ModelType::NeuralSynth,
        };

        assert_eq!(model.name(), "test_model");
        assert_eq!(model.model_type(), ModelType::NeuralSynth);
    }

    #[test]
    fn test_synth_handle_missing_model() {
        let neural = NeuralSystem::builder().build().unwrap();
        let model = NeuralModel {
            id: NeuralModelId::new(),
            name: "nonexistent".to_string(),
            model_type: ModelType::NeuralSynth,
        };

        let result = neural.synth().build_voice(&model);
        assert!(result.is_err());
    }

    #[test]
    fn test_effect_handle_missing_model() {
        let neural = NeuralSystem::builder().build().unwrap();
        let model = NeuralModel {
            id: NeuralModelId::new(),
            name: "nonexistent".to_string(),
            model_type: ModelType::Effect,
        };

        let result = neural.effects().build_effect(&model);
        assert!(result.is_err());
    }
}
