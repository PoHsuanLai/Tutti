//! Sync-safe neural effect builder wrapper
//!
//! Wraps the non-Sync EffectBuilder in a Send+Sync interface.
//! The actual Burn inference engine runs on a dedicated thread,
//! and this wrapper communicates via channels.

use crate::error::Result;
use crate::gpu::{NeuralInferenceEngine, NeuralModelId};
use std::sync::Arc;
use burn::tensor::backend::Backend;
use tutti_core::AudioUnit;

/// Sync-safe wrapper around neural effect builder
///
/// **Architecture**:
/// - EffectBuilder is created ON a dedicated inference thread
/// - SyncEffectBuilder provides a Sync interface via message passing
/// - Each effect instance gets its own pair of audio channels
///
/// The Burn models are NOT thread-safe (not Send/Sync), so they must be
/// created and used on a single thread. This wrapper manages that thread.
///
/// ## Effect inference loop
///
/// Unlike synths (which receive MIDI → produce control params), effects:
/// 1. Receive raw audio from the effect node (audio thread → inference thread)
/// 2. Run the model on the audio
/// 3. Send processed audio back (inference thread → audio thread)
///
/// The NeuralEffectNode handles the audio thread side (collecting input,
/// outputting processed audio). This builder wires up the inference side.
pub struct SyncEffectBuilder {
    /// Model name
    model_name: String,

    /// Model ID
    model_id: NeuralModelId,

    /// Processing buffer size in samples (= latency)
    buffer_size: usize,

    /// Sender for effect build requests
    build_tx: crossbeam_channel::Sender<BuildRequest>,
}

/// Request to build a new effect instance
struct BuildRequest {
    /// Response channel to send the built AudioUnit
    response_tx: crossbeam_channel::Sender<Box<dyn AudioUnit>>,
}

impl SyncEffectBuilder {
    /// Create a new sync-safe neural effect builder
    ///
    /// Spawns a dedicated thread and creates the EffectBuilder ON that thread.
    ///
    /// # Arguments
    /// * `engine_factory` - Function to create the inference engine ON the inference thread
    /// * `model_path` - Path to the neural effect model file
    /// * `buffer_size` - Processing buffer size in samples (determines latency)
    /// * `sample_rate` - Audio sample rate
    pub fn new<B: Backend + 'static, F>(
        engine_factory: F,
        model_path: impl Into<String>,
        buffer_size: usize,
        sample_rate: f32,
    ) -> Result<Self>
    where
        F: FnOnce() -> Result<Arc<NeuralInferenceEngine<B>>> + Send + 'static,
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        let model_path = model_path.into();

        let (build_tx, build_rx) = crossbeam_channel::unbounded::<BuildRequest>();

        // Channel to receive initialization result
        let (init_tx, init_rx) = crossbeam_channel::bounded::<Result<(String, NeuralModelId)>>(1);

        let init_buffer_size = buffer_size;
        let init_sample_rate = sample_rate;

        std::thread::spawn(move || {
            use super::effect_builder::EffectBuilder;

            tracing::info!("Neural effect inference thread starting for model: {}", model_path);

            // Create engine ON this thread
            let engine = match engine_factory() {
                Ok(e) => e,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            // Create builder ON this thread
            let builder = match EffectBuilder::new(engine, &model_path, init_buffer_size, init_sample_rate) {
                Ok(b) => {
                    let model_name = b.name().to_string();
                    let model_id = b.model_id();
                    let _ = init_tx.send(Ok((model_name.clone(), model_id)));
                    tracing::info!("Neural effect builder initialized: {}", model_name);
                    b
                }
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            // Main loop: handle build requests
            // Effect inference happens inside the NeuralEffectNode's channels.
            // The builder only needs to create new instances on demand.
            loop {
                match build_rx.recv() {
                    Ok(request) => {
                        match builder.build_effect() {
                            Ok(unit) => {
                                let _ = request.response_tx.send(unit);
                            }
                            Err(e) => {
                                tracing::error!("Failed to build neural effect: {:?}", e);
                            }
                        }
                    }
                    Err(_) => {
                        // All senders dropped — shut down
                        tracing::info!("Neural effect inference thread shutting down");
                        break;
                    }
                }
            }
        });

        // Wait for initialization
        let (model_name, model_id) = init_rx
            .recv()
            .map_err(|_| crate::error::Error::InferenceThreadInit)??;

        Ok(Self {
            model_name,
            model_id,
            buffer_size,
            build_tx,
        })
    }

    /// Get model name
    pub fn name(&self) -> &str {
        &self.model_name
    }

    /// Get model ID
    pub fn model_id(&self) -> NeuralModelId {
        self.model_id
    }

    /// Get the processing latency in samples
    pub fn latency(&self) -> usize {
        self.buffer_size
    }

    /// Build an effect instance (sends request to inference thread)
    pub fn build_effect_sync(&self) -> Result<Box<dyn AudioUnit>> {
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);

        let request = BuildRequest { response_tx };
        self.build_tx
            .send(request)
            .map_err(|_| crate::error::Error::InferenceThreadSend)?;

        response_rx
            .recv()
            .map_err(|_| crate::error::Error::InferenceThreadRecv)
    }
}

// SyncEffectBuilder is Send+Sync because it only holds channels and metadata
unsafe impl Send for SyncEffectBuilder {}
unsafe impl Sync for SyncEffectBuilder {}

// Implement NeuralEffectBuilder for the sync wrapper
impl tutti_core::neural::NeuralEffectBuilder for SyncEffectBuilder {
    fn build_effect(&self) -> tutti_core::Result<Box<dyn AudioUnit>> {
        self.build_effect_sync()
            .map_err(|e| tutti_core::Error::InvalidConfig(format!("Failed to build neural effect: {}", e)))
    }

    fn name(&self) -> &str {
        self.name()
    }

    fn model_id(&self) -> NeuralModelId {
        self.model_id
    }

    fn latency(&self) -> usize {
        self.buffer_size
    }
}
