//! Builder for creating neural effect instances (private implementation)
//!
//! This is an internal implementation detail used only on the inference thread.
//! External code should use `SyncEffectBuilder` which handles threading safely.

use crate::error::{Error, Result};
use crate::gpu::{NeuralInferenceEngine, NeuralModelId, ModelType, NeuralEffectNode};
use std::sync::Arc;
use burn::tensor::backend::Backend;

/// Builder for creating neural effect instances (internal use only)
///
/// **IMPORTANT**: This builder is NOT thread-safe and must only be used
/// on the dedicated inference thread. Use `SyncEffectBuilder` for public API.
///
/// Unlike `NeuralSynthBuilder` (which creates synth sources), this creates
/// effect processors that take audio in and produce processed audio out.
pub(crate) struct EffectBuilder<B: Backend> {
    /// Neural inference engine (lives on inference thread)
    engine: Arc<NeuralInferenceEngine<B>>,

    /// Path to the loaded model
    model_path: String,

    /// Loaded model ID in the engine
    model_id: NeuralModelId,

    /// Model name (extracted from path)
    model_name: String,

    /// Processing buffer size in samples (determines latency)
    buffer_size: usize,

    /// Sample rate
    sample_rate: f32,
}

impl<B: Backend> EffectBuilder<B> {
    /// Create a new effect builder
    ///
    /// Loads the model into the neural inference engine.
    pub fn new(
        engine: Arc<NeuralInferenceEngine<B>>,
        model_path: impl Into<String>,
        buffer_size: usize,
        sample_rate: f32,
    ) -> Result<Self>
    where
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        let model_path = model_path.into();

        tracing::info!("Loading neural effect model: {}", model_path);

        let model_id = engine
            .load_model(&model_path, ModelType::Effect)
            .map_err(|e| Error::ModelLoad(format!("Failed to load effect model: {}", e)))?;

        tracing::info!("Neural effect model loaded: ID {}", model_id.as_u64());

        let model_name = std::path::Path::new(&model_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Unknown")
            .to_string();

        Ok(Self {
            engine,
            model_path,
            model_id,
            model_name,
            buffer_size,
            sample_rate,
        })
    }

    /// Get the loaded model ID
    pub fn model_id(&self) -> NeuralModelId {
        self.model_id
    }

    /// Get model name
    pub fn name(&self) -> &str {
        &self.model_name
    }

}

impl<B: Backend + 'static> EffectBuilder<B> {
    /// Build a new neural effect instance
    ///
    /// Returns a `NeuralEffectNode` wrapped as `AudioUnit`. Each call creates
    /// a fresh pair of audio channels for the instance.
    pub fn build_effect(&self) -> Result<Box<dyn tutti_core::AudioUnit>> {
        tracing::info!(
            "Building neural effect instance (model ID: {}, buffer: {})",
            self.model_id.as_u64(),
            self.buffer_size,
        );

        let node = NeuralEffectNode::new(
            self.model_id,
            self.buffer_size,
        )
        .with_sample_rate(self.sample_rate);

        Ok(Box::new(node))
    }
}

impl<B: Backend> Drop for EffectBuilder<B> {
    fn drop(&mut self) {
        tracing::info!("Unloading neural effect model: {}", self.model_path);

        if let Err(e) = self.engine.unload_model(self.model_id) {
            tracing::error!("Failed to unload effect model: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::InferenceConfig;
    use crate::backend::BackendPool;
    use burn::backend::ndarray::NdArray;

    #[test]
    fn test_effect_builder_model_load_fails_gracefully() {
        let backend_pool = Arc::new(BackendPool::new().unwrap());
        let cpu_device = backend_pool.cpu_device();

        let engine = Arc::new(
            NeuralInferenceEngine::<NdArray>::new(
                cpu_device.clone(),
                InferenceConfig::default(),
            )
            .unwrap(),
        );

        // Should fail gracefully with nonexistent model file
        let result = EffectBuilder::new(engine, "nonexistent.onnx", 512, 44100.0);
        assert!(result.is_err());
    }
}
