//! Builder for creating neural synth instances (private implementation)
//!
//! This is an internal implementation detail used only on the inference thread.
//! External code should use `SyncNeuralSynthBuilder` which handles threading safely.

use super::neural_synth::NeuralSynth;
use crate::error::Result;
use crate::gpu::{InferenceRequest, ModelType, NeuralInferenceEngine, NeuralModelId, VoiceId};
use burn::tensor::backend::Backend;
use std::sync::Arc;
use tutti_core::AudioUnit;

/// Builder for creating neural synth instances (internal use only)
///
/// **IMPORTANT**: This builder is NOT thread-safe and must only be used
/// on the dedicated inference thread. Use `SyncNeuralSynthBuilder` for public API.
///
/// **Architecture**:
/// 1. Loads neural model into inference engine
/// 2. Creates parameter queue for each track
/// 3. Builds NeuralSynth instances that read from queue
///
/// The builder lives entirely on the inference thread, just like a VST
/// plugin lives on the audio thread.
pub(crate) struct NeuralSynthBuilder<B: Backend> {
    /// Neural inference engine (lives on inference thread, NOT shared)
    engine: Arc<NeuralInferenceEngine<B>>,

    /// Path to the loaded model
    #[allow(dead_code)]
    model_path: String,

    /// Loaded model ID in the engine
    model_id: NeuralModelId,

    /// Model name (extracted from path)
    model_name: String,

    /// Sample rate for synth instances
    sample_rate: f32,

    /// Audio buffer size in samples
    buffer_size: usize,

    /// Voice counter for assigning voice IDs
    next_track_id: std::sync::atomic::AtomicU32,

    /// MIDI sender channel (Phase 2)
    /// Passed to each NeuralSynth instance for sending MIDI to inference thread
    midi_tx: crossbeam_channel::Sender<InferenceRequest>,
}

impl<B: Backend> NeuralSynthBuilder<B> {
    /// Create a new DDSP synth builder
    ///
    /// Loads the model into the neural inference engine.
    ///
    /// # Arguments
    /// * `engine` - Shared neural inference engine (GPU or CPU)
    /// * `model_path` - Path to ONNX/Burn model file
    /// * `sample_rate` - Audio sample rate
    /// * `midi_tx` - MIDI sender channel for voices to send MIDI events
    ///
    /// # Returns
    /// Builder that can create synth instances for multiple voices
    pub fn new(
        engine: Arc<NeuralInferenceEngine<B>>,
        model_path: impl Into<String>,
        sample_rate: f32,
        buffer_size: usize,
        midi_tx: crossbeam_channel::Sender<InferenceRequest>,
    ) -> Result<Self>
    where
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        let model_path = model_path.into();

        // Load model into engine
        let model_id = engine.load_model(&model_path, ModelType::NeuralSynth)?;

        // Extract model name from path
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
            sample_rate,
            buffer_size,
            next_track_id: std::sync::atomic::AtomicU32::new(0),
            midi_tx,
        })
    }

    /// Get the loaded model ID
    pub fn model_id(&self) -> NeuralModelId {
        self.model_id
    }
}

impl<B: Backend + 'static> NeuralSynthBuilder<B> {
    /// Build a new DDSP synth instance
    ///
    /// Returns an AudioUnit (not Plugin) - neural synths don't need
    /// traditional plugin features like parameters/presets.
    pub fn build_synth(&self) -> Result<Box<dyn AudioUnit>> {
        // Assign next voice ID
        let track_id: VoiceId =
            self.next_track_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst) as u64;

        // Create lock-free parameter queue for this voice
        let param_queue = self.engine.create_queue(track_id);

        // Create synth instance with MIDI sender
        let synth = NeuralSynth::new(
            track_id,
            self.model_id,
            param_queue,
            self.sample_rate,
            self.buffer_size,
            self.midi_tx.clone(),
        );

        Ok(Box::new(synth))
    }

    /// Get model name
    pub fn name(&self) -> &str {
        &self.model_name
    }
}

// NOTE: NeuralSynthBuilder does NOT implement NeuralSynthBuilder trait
// because it's not thread-safe and should only be used internally.
// Use SyncNeuralSynthBuilder for the public API (which does implement the trait).

impl<B: Backend> Drop for NeuralSynthBuilder<B> {
    fn drop(&mut self) {
        // Unload model from engine
        let _ = self.engine.unload_model(self.model_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendPool;
    use crate::gpu::InferenceConfig;
    use burn::backend::ndarray::NdArray;

    #[test]
    fn test_builder_creation() {
        // Create CPU backend for testing
        let backend_pool = Arc::new(BackendPool::new().unwrap());
        let cpu_device = backend_pool.cpu_device();

        // Create inference engine (verifying it constructs without error)
        #[allow(clippy::arc_with_non_send_sync)] // NdArray is single-threaded by design
        let _engine = Arc::new(
            NeuralInferenceEngine::<NdArray>::new(cpu_device.clone(), InferenceConfig::default())
                .unwrap(),
        );

        // Create builder (model loading will fail without real model file, but constructor should work)
        // In a real test, we'd use a fixture model file
        // let builder = NeuralSynthBuilder::new(engine, "test_model.onnx", 44100.0);
        // assert!(builder.is_err()); // Expected to fail without real file
    }
}
