//! Sync-safe Neural synth builder wrapper
//!
//! Wraps the non-Sync NeuralSynthBuilder in a Send+Sync interface.
//! The actual Burn inference engine runs on a dedicated thread,
//! and this wrapper communicates via lock-free channels.

use crate::error::Result;
use crate::gpu::{NeuralInferenceEngine, NeuralModelId, VoiceId};
use burn::tensor::backend::Backend;
use std::sync::Arc;
use tutti_core::neural::BatchingStrategy;
use tutti_core::AudioUnit;

/// Push inference response params to a voice's parameter queue
///
/// Used by the inference thread loop after single or batched inference.
fn push_response_to_voice<B: Backend>(
    engine: &NeuralInferenceEngine<B>,
    track_id: VoiceId,
    params: crate::gpu::ControlParams,
) {
    if let Some(sender) = engine.get_sender(track_id) {
        // Drop params if queue is full (non-blocking)
        let _ = sender.try_send(params);
    }
}

/// Sync-safe wrapper around Neural synth builder
///
/// **Architecture**:
/// - NeuralSynthBuilder is created ON a dedicated inference thread (not moved to it)
/// - SyncNeuralSynthBuilder provides a Sync interface via message passing
/// - Each voice gets its own lock-free queue for control params
///
/// The Burn models are NOT thread-safe (not Send/Sync), so they must be
/// created and used on a single thread. This wrapper manages that thread
/// and allows the audio backend to treat neural synths as Sync.
pub struct SyncNeuralSynthBuilder {
    /// Model name
    model_name: String,

    /// Model ID
    model_id: NeuralModelId,

    /// Audio buffer size in samples
    buffer_size: usize,

    /// Sender for voice build requests
    build_tx: crossbeam_channel::Sender<BuildRequest>,

    /// Sender for MIDI events (Phase 2)
    /// Audio thread sends MIDI → Inference thread for neural processing
    midi_tx: crossbeam_channel::Sender<crate::gpu::InferenceRequest>,
}

/// Request to build a new voice
struct BuildRequest {
    /// Response channel to send the built processor
    response_tx: crossbeam_channel::Sender<Box<dyn AudioUnit>>,
}

impl SyncNeuralSynthBuilder {
    /// Create a new sync-safe Neural synth builder
    ///
    /// Spawns a dedicated thread and creates the NeuralSynthBuilder ON that thread.
    ///
    /// # Arguments
    /// * `engine_factory` - Function to create the neural inference engine ON the inference thread
    /// * `model_path` - Path to the Neural synth model file
    /// * `sample_rate` - Audio sample rate
    ///
    /// # Returns
    /// A Sync-safe wrapper that can be shared across threads
    ///
    /// # Example
    /// ```ignore
    /// let builder = SyncNeuralSynthBuilder::new(
    ///     || {
    ///         let backend_pool = BackendPool::new()?;
    ///         let device = backend_pool.cpu_device();
    ///         Arc::new(NeuralInferenceEngine::new(device, InferenceConfig::default())?)
    ///     },
    ///     "model.onnx",

    /// Create with an optional strategy receiver for graph-aware batching.
    pub fn new_with_strategy<B: Backend + 'static, F>(
        engine_factory: F,
        model_path: impl Into<String>,
        sample_rate: f32,
        buffer_size: usize,
        strategy_rx: Option<crossbeam_channel::Receiver<BatchingStrategy>>,
    ) -> Result<Self>
    where
        F: FnOnce() -> Result<Arc<NeuralInferenceEngine<B>>> + Send + 'static,
        B::FloatElem: burn::serde::de::DeserializeOwned,
    {
        let model_path = model_path.into();

        // Create channel for build requests
        let (build_tx, build_rx) = crossbeam_channel::unbounded::<BuildRequest>();

        // Create channel for MIDI events
        let (midi_tx, midi_rx) = crossbeam_channel::unbounded::<crate::gpu::InferenceRequest>();

        // Clone midi_tx for the spawned thread (keep one copy for return value)
        let midi_tx_for_thread = midi_tx.clone();

        // Clone strategy_rx for the thread
        let strategy_rx_for_thread = strategy_rx.clone();

        // Channel to receive initialization result
        let (init_tx, init_rx) = crossbeam_channel::bounded::<Result<(String, NeuralModelId)>>(1);

        // Spawn inference thread - everything is created ON this thread
        std::thread::spawn(move || {
            use super::neural_synth_builder::NeuralSynthBuilder;

            // Create engine ON this thread
            let engine = match engine_factory() {
                Ok(e) => e,
                Err(e) => {
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            // Clone Arc for builder (builder takes ownership, loop uses reference)
            let engine_for_loop = Arc::clone(&engine);

            // Create builder ON this thread (Burn models can't be moved between threads)
            let builder = match NeuralSynthBuilder::new(
                engine,
                &model_path,
                sample_rate,
                buffer_size,
                midi_tx_for_thread.clone(),
            ) {
                Ok(b) => {
                    let model_name = b.name().to_string();
                    let model_id = b.model_id();

                    // Send initialization success
                    let _ = init_tx.send(Ok((model_name.clone(), model_id)));
                    b
                }
                Err(e) => {
                    // Send initialization failure
                    let _ = init_tx.send(Err(e));
                    return;
                }
            };

            let use_graph_aware = strategy_rx_for_thread.is_some();

            if use_graph_aware {
                Self::inference_loop_graph_aware(
                    engine_for_loop,
                    builder,
                    build_rx,
                    midi_rx,
                    strategy_rx_for_thread.unwrap(),
                );
            } else {
                Self::inference_loop_timing_based(engine_for_loop, builder, build_rx, midi_rx);
            }
        });

        // Wait for initialization to complete
        let (model_name, model_id) = init_rx
            .recv()
            .map_err(|_| crate::error::Error::InferenceThreadInit)??;

        Ok(Self {
            model_name,
            model_id,
            buffer_size,
            build_tx,
            midi_tx,
        })
    }

    /// Timing-based inference loop (original BatchCollector approach).
    ///
    /// Collects requests until batch_size OR timeout, then fires.
    fn inference_loop_timing_based<B: Backend + 'static>(
        engine: Arc<NeuralInferenceEngine<B>>,
        builder: super::neural_synth_builder::NeuralSynthBuilder<B>,
        build_rx: crossbeam_channel::Receiver<BuildRequest>,
        midi_rx: crossbeam_channel::Receiver<crate::gpu::InferenceRequest>,
    ) {
        use crate::gpu::batch::BatchCollector;

        let batch_config = engine.config();
        let mut batch_collector = BatchCollector::new(
            batch_config.batch_size,
            1, // 1ms max wait
        );
        let mut pending_requests: Vec<crate::gpu::InferenceRequest> = Vec::new();

        loop {
            // 1. Check for build requests (non-blocking)
            if let Ok(request) = build_rx.try_recv() {
                if let Ok(unit) = builder.build_synth() {
                    let _ = request.response_tx.send(unit);
                }
            }

            // 2. Collect MIDI events into batch
            while let Ok(inference_request) = midi_rx.try_recv() {
                if pending_requests.is_empty() {
                    batch_collector.start_batch();
                }
                pending_requests.push(inference_request);
                if batch_collector.is_ready(pending_requests.len()) {
                    break;
                }
            }

            // 3. Process batch when ready (full OR timeout)
            if !pending_requests.is_empty() && batch_collector.is_ready(pending_requests.len()) {
                let batch = std::mem::take(&mut pending_requests);
                let batch_size = batch.len();
                batch_collector.reset();

                Self::execute_batch(&engine, batch, batch_size);
            }

            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }

    /// Graph-aware inference loop.
    ///
    /// Uses `GraphAwareBatcher` which groups requests by model and respects
    /// graph dependencies. Receives strategy updates from `NeuralSystem`.
    fn inference_loop_graph_aware<B: Backend + 'static>(
        engine: Arc<NeuralInferenceEngine<B>>,
        builder: super::neural_synth_builder::NeuralSynthBuilder<B>,
        build_rx: crossbeam_channel::Receiver<BuildRequest>,
        midi_rx: crossbeam_channel::Receiver<crate::gpu::InferenceRequest>,
        strategy_rx: crossbeam_channel::Receiver<BatchingStrategy>,
    ) {
        use crate::gpu::GraphAwareBatcher;

        let mut batcher = GraphAwareBatcher::new(Arc::clone(&engine));
        let mut last_stats_log = std::time::Instant::now();

        loop {
            // 1. Check for build requests (non-blocking)
            if let Ok(request) = build_rx.try_recv() {
                if let Ok(unit) = builder.build_synth() {
                    let _ = request.response_tx.send(unit);
                }
            }

            // 2. Check for strategy updates (non-blocking)
            //    Drain channel to get the latest strategy
            let mut latest_strategy = None;
            while let Ok(strategy) = strategy_rx.try_recv() {
                latest_strategy = Some(strategy);
            }
            if let Some(strategy) = latest_strategy {
                batcher.set_strategy(strategy);
            }

            // 3. Queue incoming inference requests
            while let Ok(request) = midi_rx.try_recv() {
                batcher.queue_request(request);
            }

            // 4. Process all pending batches
            if batcher.has_pending() {
                if let Ok(responses) = batcher.process_batches() {
                    for response in responses {
                        push_response_to_voice(&engine, response.track_id, response.params);
                    }
                }
            }

            // 5. Periodic stats logging (every 10 seconds)
            if last_stats_log.elapsed() > std::time::Duration::from_secs(10) {
                let stats = batcher.stats();
                if stats.requests_processed > 0 {
                    tracing::debug!(
                        "Neural batcher stats: {} requests, {} batches, avg batch {:.1}, latency {:.2}ms/req ({:.2}ms/batch), strategy: {}",
                        stats.requests_processed,
                        stats.batches_processed,
                        stats.avg_batch_size(),
                        stats.avg_latency_per_request(),
                        stats.avg_latency_per_batch(),
                        batcher.strategy().map_or("none".to_string(), |s| {
                            format!("{} models", s.model_count())
                        }),
                    );
                }
                last_stats_log = std::time::Instant::now();
            }

            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }

    /// Execute a batch of inference requests (shared between both loops).
    fn execute_batch<B: Backend>(
        engine: &NeuralInferenceEngine<B>,
        batch: Vec<crate::gpu::InferenceRequest>,
        batch_size: usize,
    ) {
        if batch_size == 1 {
            let request = batch.into_iter().next().unwrap();
            let track_id = request.track_id;
            if let Ok(response) = engine.infer(request) {
                push_response_to_voice(engine, track_id, response.params);
            }
        } else {
            if let Ok(responses) = engine.infer_batch(batch) {
                for response in responses {
                    push_response_to_voice(engine, response.track_id, response.params);
                }
            }
        }
    }

    /// Get model name
    pub fn name(&self) -> &str {
        &self.model_name
    }

    /// Get model ID
    pub fn model_id(&self) -> NeuralModelId {
        self.model_id
    }

    /// Build a voice (sends request to inference thread)
    pub fn build_voice_sync(&self) -> Result<Box<dyn AudioUnit>> {
        // Create response channel
        let (response_tx, response_rx) = crossbeam_channel::bounded(1);

        // Send build request
        let request = BuildRequest { response_tx };
        self.build_tx
            .send(request)
            .map_err(|_| crate::error::Error::InferenceThreadSend)?;

        // Wait for response
        response_rx
            .recv()
            .map_err(|_| crate::error::Error::InferenceThreadRecv)
    }

    /// Send pre-computed features for neural inference.
    ///
    /// The caller is responsible for maintaining a `MidiState` per voice,
    /// applying MIDI events to it, and calling `to_features()`.
    ///
    /// RT-safe and non-blocking. Drops request if queue is full.
    ///
    /// # Returns
    /// `true` if queued successfully, `false` if queue is full
    pub fn send_features_rt(&self, track_id: VoiceId, features: Vec<f32>) -> bool {
        let request = crate::gpu::InferenceRequest {
            track_id,
            model_id: self.model_id,
            features,
            buffer_size: self.buffer_size,
        };

        self.midi_tx.try_send(request).is_ok()
    }
}

// SyncNeuralSynthBuilder is Send+Sync because it only holds channels and metadata
unsafe impl Send for SyncNeuralSynthBuilder {}
unsafe impl Sync for SyncNeuralSynthBuilder {}

// Implement NeuralSynthBuilder for the sync wrapper
impl tutti_core::neural::NeuralSynthBuilder for SyncNeuralSynthBuilder {
    fn build_voice(&self) -> tutti_core::Result<Box<dyn AudioUnit>> {
        self.build_voice_sync().map_err(|e| {
            tutti_core::Error::InvalidConfig(format!("Failed to build neural synth voice: {}", e))
        })
    }

    fn name(&self) -> &str {
        self.name()
    }

    fn model_id(&self) -> tutti_core::neural::NeuralModelId {
        self.model_id
    }
}
