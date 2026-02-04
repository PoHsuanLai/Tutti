//! Unified neural inference engine — single thread for all models.
//!
//! Replaces N+1 threads (1 per synth model + 1 for effects) with a single
//! inference thread. All AudioUnit wrappers (synths, effects) submit tensor
//! requests through a shared channel and get results back via their own
//! response channels.
//!
//! The engine has a pure tensor API — it doesn't know whether a request
//! comes from a synth or an effect. It just runs `forward(model_id, tensor)`.
//!
//! ## Why factory closures?
//!
//! Burn models are `Send` but not `Sync` (they hold `Arc<dyn Fn + Send>`).
//! We can't send a `NeuralModel` across threads via a channel because
//! `Arc<T>` requires `T: Send + Sync` to be `Send` itself.
//!
//! Instead, callers send a **factory closure** `Box<dyn FnOnce() -> NeuralModel + Send>`
//! which gets executed on the engine thread. The model never leaves that thread.

use crate::backend::BackendPool;
use crate::error::{Error, Result};
use crate::gpu::batch::BatchCollector;
use crate::gpu::engine::{InferenceConfig, NeuralInferenceEngine, NeuralModelId};
use crate::gpu::fusion::NeuralModel;
use crate::gpu::{ControlParams, SharedEffectAudioQueue};

use std::collections::HashMap;
use tutti_core::BatchingStrategy;

use burn::backend::NdArray;
use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ============================================================================
// Public types
// ============================================================================

/// A tensor submission to the inference engine.
pub struct TensorRequest {
    pub model_id: NeuralModelId,
    pub input: Vec<f32>,
    pub input_shape: [usize; 2], // [batch, features]
    pub response: ResponseChannel,
}

/// How to deliver the inference result back to the caller.
pub enum ResponseChannel {
    /// Synth path: convert tensor → ControlParams → push to channel.
    Params {
        sender: Sender<ControlParams>,
        buffer_size: usize,
    },
    /// Effect path: write processed audio to the double-buffered queue.
    Audio(SharedEffectAudioQueue),
    /// Generic: return raw tensor data via a oneshot channel.
    OneShot(Sender<Vec<f32>>),
}

/// Factory that creates a NeuralModel on the engine thread.
///
/// The closure runs on the engine thread where the Burn device lives.
/// This avoids sending non-Sync models across threads.
type ModelFactory = Box<dyn FnOnce() -> NeuralModel<NdArray> + Send>;

// ============================================================================
// Internal types
// ============================================================================

/// Commands sent to the engine thread.
enum EngineCommand {
    /// Register a new model via a factory closure.
    RegisterModel {
        factory: ModelFactory,
        response_tx: Sender<NeuralModelId>,
    },
    /// Update the batching strategy from the graph analyzer.
    UpdateStrategy(BatchingStrategy),
    /// Shutdown the engine thread.
    Shutdown,
}

// ============================================================================
// NeuralEngine
// ============================================================================

/// Unified neural engine handle.
///
/// Owns a single inference thread. All AudioUnit instances (synths, effects)
/// share the same `request_tx` to submit tensor requests.
pub struct NeuralEngine {
    cmd_tx: Sender<EngineCommand>,
    request_tx: Sender<TensorRequest>,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NeuralEngine {
    /// Create and start the unified inference engine.
    ///
    /// Spawns a single dedicated thread that:
    /// 1. Processes commands (register model, shutdown)
    /// 2. Drains tensor requests from the shared channel
    /// 3. Groups by model_id → batched forward()
    /// 4. Sends results back via each request's ResponseChannel
    pub fn start(config: InferenceConfig) -> Result<Self> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<EngineCommand>(16);
        let (request_tx, request_rx) = crossbeam_channel::bounded::<TensorRequest>(256);
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let thread = std::thread::Builder::new()
            .name("neural-engine".into())
            .spawn(move || {
                if let Err(e) = inference_loop(config, cmd_rx, request_rx, &running_clone) {
                    tracing::error!("Neural engine thread failed: {}", e);
                }
                running_clone.store(false, Ordering::Release);
            })
            .map_err(|e| Error::Inference(format!("Failed to spawn engine thread: {}", e)))?;

        Ok(Self {
            cmd_tx,
            request_tx,
            running,
            thread: Some(thread),
        })
    }

    /// Register a model with the engine via a factory closure.
    ///
    /// The factory runs on the engine thread (where the Burn device lives).
    /// Blocks until the engine thread creates the model and returns the ID.
    pub fn register_model(
        &self,
        factory: impl FnOnce() -> NeuralModel<NdArray> + Send + 'static,
    ) -> Result<NeuralModelId> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.cmd_tx
            .send(EngineCommand::RegisterModel {
                factory: Box::new(factory),
                response_tx: tx,
            })
            .map_err(|_| Error::InferenceThreadSend)?;
        rx.recv().map_err(|_| Error::InferenceThreadRecv)
    }

    /// Get a clone of the request sender for AudioUnit instances.
    pub fn request_sender(&self) -> Sender<TensorRequest> {
        self.request_tx.clone()
    }

    /// Check if the engine thread is still running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    /// Update the batching strategy from graph analysis.
    ///
    /// Best-effort: silently drops if the command channel is full.
    /// The engine works fine without a strategy (falls back to arrival-order grouping).
    pub fn update_strategy(&self, strategy: BatchingStrategy) {
        let _ = self
            .cmd_tx
            .try_send(EngineCommand::UpdateStrategy(strategy));
    }

    /// Shut down the engine thread and wait for it to finish.
    pub fn shutdown(&mut self) {
        let _ = self.cmd_tx.send(EngineCommand::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NeuralEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

// ============================================================================
// Inference loop (runs on the dedicated engine thread)
// ============================================================================

fn inference_loop(
    config: InferenceConfig,
    cmd_rx: Receiver<EngineCommand>,
    request_rx: Receiver<TensorRequest>,
    running: &AtomicBool,
) -> Result<()> {
    // Initialize the engine on this thread (Burn backends are not Sync)
    let pool = BackendPool::new()?;
    let device = (**pool.cpu_device()).clone();
    let mut engine = NeuralInferenceEngine::<NdArray>::new(device, config)?;
    let mut batch_collector = BatchCollector::new(engine.config().batch_size, 2);

    // Scratch buffer for collecting requests
    let mut pending: Vec<TensorRequest> = Vec::with_capacity(64);

    // Model priority from graph analysis (lower = process first)
    let mut model_priority: HashMap<NeuralModelId, usize> = HashMap::new();

    tracing::info!("Neural engine thread started");

    while running.load(Ordering::Acquire) {
        // 1. Process commands (non-blocking)
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                EngineCommand::RegisterModel {
                    factory,
                    response_tx,
                } => {
                    let model = factory();
                    let id = engine.register_model(model);
                    let _ = response_tx.send(id);
                }
                EngineCommand::UpdateStrategy(strategy) => {
                    model_priority = compute_model_priority(&strategy);
                    tracing::debug!("Updated batching strategy: {} models", model_priority.len());
                }
                EngineCommand::Shutdown => {
                    running.store(false, Ordering::Release);
                    tracing::info!("Neural engine shutting down");
                    return Ok(());
                }
            }
        }

        // 2. Drain all pending tensor requests
        while let Ok(req) = request_rx.try_recv() {
            if pending.is_empty() {
                batch_collector.start_batch();
            }
            pending.push(req);
        }

        // 3. Process batch if ready
        if !pending.is_empty() && batch_collector.is_ready(pending.len()) {
            process_batch(&mut engine, &mut pending, &model_priority);
            batch_collector.reset();
        }

        // 4. If nothing to do, brief sleep to avoid busy-waiting
        if pending.is_empty() {
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }

    Ok(())
}

/// Compute per-model execution priority from the batching strategy.
///
/// For each model, takes the minimum execution order of its nodes.
/// Lower priority = should be processed first (upstream in the graph).
fn compute_model_priority(strategy: &BatchingStrategy) -> HashMap<NeuralModelId, usize> {
    strategy
        .model_batches
        .iter()
        .map(|(model_id, nodes)| {
            let min_order = nodes
                .iter()
                .filter_map(|n| strategy.execution_order.get(n))
                .min()
                .copied()
                .unwrap_or(usize::MAX);
            (*model_id, min_order)
        })
        .collect()
}

/// Process a batch of tensor requests.
///
/// If model priority is available (from graph analysis), sorts requests so that
/// upstream models are processed first, reducing pipeline stalls for dependent nodes.
fn process_batch(
    engine: &mut NeuralInferenceEngine<NdArray>,
    pending: &mut Vec<TensorRequest>,
    model_priority: &HashMap<NeuralModelId, usize>,
) {
    let mut requests: Vec<_> = pending.drain(..).collect();

    // Sort by model execution priority (lower = upstream, process first)
    if !model_priority.is_empty() {
        requests.sort_by_key(|r| *model_priority.get(&r.model_id).unwrap_or(&usize::MAX));
    }

    let forward_requests: Vec<(NeuralModelId, Vec<f32>, usize)> = requests
        .iter()
        .map(|r| (r.model_id, r.input.clone(), r.input_shape[1]))
        .collect();

    match engine.forward_grouped(&forward_requests) {
        Ok(results) => {
            for (req, result) in requests.into_iter().zip(results.into_iter()) {
                dispatch_result(req, result);
            }
        }
        Err(e) => {
            tracing::error!("Batched forward failed: {}", e);
            for req in requests {
                dispatch_result(req, vec![]);
            }
        }
    }
}

/// Send inference result back through the request's response channel.
fn dispatch_result(req: TensorRequest, result: Vec<f32>) {
    match req.response {
        ResponseChannel::Params {
            sender,
            buffer_size,
        } => {
            let params = tensor_to_control_params(&result, buffer_size);
            let _ = sender.try_send(params);
        }
        ResponseChannel::Audio(queue) => {
            queue.write_output(&result);
        }
        ResponseChannel::OneShot(tx) => {
            let _ = tx.send(result);
        }
    }
}

/// Convert flat tensor output to ControlParams.
///
/// Expected layout: [f0_0, ..., f0_n, amp_0, ..., amp_n] where n = buffer_size.
fn tensor_to_control_params(data: &[f32], buffer_size: usize) -> ControlParams {
    if data.len() >= buffer_size * 2 {
        ControlParams {
            f0: data[..buffer_size].to_vec(),
            amplitudes: data[buffer_size..buffer_size * 2].to_vec(),
        }
    } else if data.is_empty() {
        ControlParams::default()
    } else {
        let half = data.len() / 2;
        let mut f0 = data[..half].to_vec();
        let mut amplitudes = data[half..].to_vec();
        f0.resize(buffer_size, 440.0);
        amplitudes.resize(buffer_size, 0.0);
        ControlParams { f0, amplitudes }
    }
}

/// Submit a tensor request, dropping it if the queue is full (RT-safe).
#[inline]
pub fn submit_request(tx: &Sender<TensorRequest>, request: TensorRequest) -> bool {
    match tx.try_send(request) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) => {
            tracing::trace!("Neural request queue full, dropping request");
            false
        }
        Err(TrySendError::Disconnected(_)) => {
            tracing::warn!("Neural engine disconnected");
            false
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_start_shutdown() {
        let mut engine = NeuralEngine::start(InferenceConfig::default()).unwrap();
        assert!(engine.is_running());
        engine.shutdown();
        assert!(!engine.is_running());
    }

    #[test]
    fn test_register_model_via_factory() {
        let mut engine = NeuralEngine::start(InferenceConfig::default()).unwrap();

        let id = engine
            .register_model(|| NeuralModel::<NdArray>::from_forward(|input| input))
            .unwrap();

        assert_ne!(id.as_u64(), 0);
        engine.shutdown();
    }

    #[test]
    fn test_oneshot_inference() {
        let mut engine = NeuralEngine::start(InferenceConfig::default()).unwrap();

        let model_id = engine
            .register_model(|| NeuralModel::<NdArray>::from_forward(|input| input))
            .unwrap();

        let (tx, rx) = crossbeam_channel::bounded(1);
        let request = TensorRequest {
            model_id,
            input: vec![1.0, 2.0, 3.0, 4.0],
            input_shape: [1, 4],
            response: ResponseChannel::OneShot(tx),
        };

        engine.request_sender().send(request).unwrap();

        let result = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(result, vec![1.0, 2.0, 3.0, 4.0]);

        engine.shutdown();
    }

    #[test]
    fn test_param_response() {
        let mut engine = NeuralEngine::start(InferenceConfig::default()).unwrap();

        let model_id = engine
            .register_model(|| NeuralModel::<NdArray>::from_forward(|input| input))
            .unwrap();

        // crossbeam channel for ControlParams delivery
        let (param_tx, param_rx) = crossbeam_channel::bounded::<ControlParams>(16);

        let buffer_size = 2;
        let request = TensorRequest {
            model_id,
            input: vec![440.0, 440.0, 0.5, 0.5],
            input_shape: [1, 4],
            response: ResponseChannel::Params {
                sender: param_tx,
                buffer_size,
            },
        };

        engine.request_sender().send(request).unwrap();

        let params = param_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(params.f0, vec![440.0, 440.0]);
        assert_eq!(params.amplitudes, vec![0.5, 0.5]);

        engine.shutdown();
    }

    #[test]
    fn test_effect_response() {
        let mut engine = NeuralEngine::start(InferenceConfig::default()).unwrap();

        let model_id = engine
            .register_model(|| NeuralModel::<NdArray>::from_forward(|input| input.mul_scalar(2.0)))
            .unwrap();

        let queue = crate::gpu::shared_effect_queue(2, 2);

        queue.write_input(0, 0.1);
        queue.write_input(1, 0.2);
        queue.write_input(0, 0.3);
        queue.write_input(1, 0.4);

        let input_data = queue.take_input().unwrap().to_vec();
        let features = input_data.len();

        let request = TensorRequest {
            model_id,
            input: input_data,
            input_shape: [1, features],
            response: ResponseChannel::Audio(queue.clone()),
        };

        engine.request_sender().send(request).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(queue.has_output());

        engine.shutdown();
    }

    #[test]
    fn test_tensor_to_control_params() {
        let data = vec![440.0, 440.0, 0.5, 0.5];
        let params = tensor_to_control_params(&data, 2);
        assert_eq!(params.f0, vec![440.0, 440.0]);
        assert_eq!(params.amplitudes, vec![0.5, 0.5]);
    }

    #[test]
    fn test_tensor_to_control_params_empty() {
        let params = tensor_to_control_params(&[], 512);
        assert!(params.f0.is_empty());
        assert!(params.amplitudes.is_empty());
    }

    #[test]
    fn test_submit_request_helper() {
        let (tx, _rx) = crossbeam_channel::bounded(1);
        let request = TensorRequest {
            model_id: NeuralModelId::new(),
            input: vec![1.0],
            input_shape: [1, 1],
            response: ResponseChannel::OneShot(crossbeam_channel::bounded(1).0),
        };
        assert!(submit_request(&tx, request));
    }
}
