//! Unified neural inference engine — single thread for all models.

use crate::error::{Error, Result};
use crate::gpu::batch::BatchCollector;
use crate::gpu::{ControlParams, SharedEffectAudioQueue};

use std::collections::HashMap;
use std::sync::Arc;
use tutti_core::{
    BackendFactory, BatchingStrategy, InferenceBackend, InferenceConfig, NeuralModelId,
};

use crossbeam_channel::{Receiver, Sender, TrySendError};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct TensorRequest {
    pub model_id: NeuralModelId,
    pub input: Arc<[f32]>,
    pub input_shape: [usize; 2],
    pub response: ResponseChannel,
}

pub enum ResponseChannel {
    Params {
        sender: Sender<ControlParams>,
        buffer_size: usize,
    },
    Audio(SharedEffectAudioQueue),
    OneShot(Sender<Vec<f32>>),
}

type ModelFactory = Box<dyn FnOnce(&mut dyn InferenceBackend) -> NeuralModelId + Send>;

enum EngineCommand {
    RegisterModel {
        factory: ModelFactory,
        response_tx: Sender<NeuralModelId>,
    },
    UpdateStrategy(BatchingStrategy),
    Shutdown,
}

pub struct NeuralEngine {
    cmd_tx: Sender<EngineCommand>,
    request_tx: Sender<TensorRequest>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NeuralEngine {
    pub fn start_with(config: InferenceConfig, backend_factory: BackendFactory) -> Result<Self> {
        let (cmd_tx, cmd_rx) = crossbeam_channel::bounded::<EngineCommand>(16);
        let (request_tx, request_rx) = crossbeam_channel::bounded::<TensorRequest>(256);
        let running = Arc::new(AtomicBool::new(true));

        let thread = std::thread::Builder::new()
            .name("neural-engine".into())
            .spawn(move || {
                if let Err(e) =
                    inference_loop(config, backend_factory, cmd_rx, request_rx, &running)
                {
                    tracing::error!("Neural engine thread failed: {}", e);
                }
            })
            .map_err(|e| Error::Inference(format!("Failed to spawn engine thread: {}", e)))?;

        Ok(Self {
            cmd_tx,
            request_tx,
            thread: Some(thread),
        })
    }

    pub fn register_model(
        &self,
        factory: impl FnOnce(&mut dyn InferenceBackend) -> NeuralModelId + Send + 'static,
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

    pub fn request_sender(&self) -> Sender<TensorRequest> {
        self.request_tx.clone()
    }

    pub fn update_strategy(&self, strategy: BatchingStrategy) {
        let _ = self
            .cmd_tx
            .try_send(EngineCommand::UpdateStrategy(strategy));
    }

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

fn inference_loop(
    config: InferenceConfig,
    backend_factory: BackendFactory,
    cmd_rx: Receiver<EngineCommand>,
    request_rx: Receiver<TensorRequest>,
    running: &AtomicBool,
) -> Result<()> {
    let mut backend = backend_factory(config.clone())
        .map_err(|e| Error::Inference(format!("Backend init failed: {}", e)))?;

    let mut batch_collector = BatchCollector::new(backend.config().batch_size, 2);
    let mut pending: Vec<TensorRequest> = Vec::with_capacity(64);
    let mut model_priority: HashMap<NeuralModelId, usize> = HashMap::new();

    tracing::info!(
        "Neural engine thread started (backend: {})",
        backend.capabilities().name
    );

    while running.load(Ordering::Acquire) {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                EngineCommand::RegisterModel {
                    factory,
                    response_tx,
                } => {
                    let id = factory(backend.as_mut());
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

        while let Ok(req) = request_rx.try_recv() {
            if pending.is_empty() {
                batch_collector.start_batch();
            }
            pending.push(req);
        }

        if !pending.is_empty() && batch_collector.is_ready(pending.len()) {
            process_batch(backend.as_mut(), &mut pending, &model_priority);
            batch_collector.reset();
        }

        if pending.is_empty() {
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
    }

    Ok(())
}

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

fn process_batch(
    backend: &mut dyn InferenceBackend,
    pending: &mut Vec<TensorRequest>,
    model_priority: &HashMap<NeuralModelId, usize>,
) {
    let mut requests: Vec<_> = std::mem::take(pending);

    if !model_priority.is_empty() {
        requests.sort_by_key(|r| *model_priority.get(&r.model_id).unwrap_or(&usize::MAX));
    }

    let forward_requests: Vec<(NeuralModelId, Vec<f32>, usize)> = requests
        .iter()
        .map(|r| (r.model_id, r.input.to_vec(), r.input_shape[1]))
        .collect();

    match backend.forward_grouped(&forward_requests) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_backend_factory() -> BackendFactory {
        Box::new(|config| Ok(Box::new(SimpleTestBackend::new(config)) as Box<dyn InferenceBackend>))
    }

    struct SimpleTestBackend {
        config: InferenceConfig,
        models: HashMap<NeuralModelId, Box<dyn Fn(&[f32], [usize; 2]) -> Vec<f32> + Send>>,
    }

    impl SimpleTestBackend {
        fn new(config: InferenceConfig) -> Self {
            Self {
                config,
                models: HashMap::new(),
            }
        }
    }

    impl InferenceBackend for SimpleTestBackend {
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
            let mut results = Vec::with_capacity(requests.len());
            for (model_id, data, feat_dim) in requests {
                if let Some(f) = self.models.get(model_id) {
                    let batch = if *feat_dim > 0 {
                        data.len() / feat_dim
                    } else {
                        1
                    };
                    results.push(f(data, [batch, *feat_dim]));
                } else {
                    results.push(data.clone());
                }
            }
            Ok(results)
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
    fn test_engine_start_shutdown() {
        let mut engine =
            NeuralEngine::start_with(InferenceConfig::default(), test_backend_factory()).unwrap();
        engine.shutdown();
    }

    #[test]
    fn test_register_model_via_factory() {
        let mut engine =
            NeuralEngine::start_with(InferenceConfig::default(), test_backend_factory()).unwrap();

        let id = engine
            .register_model(|backend| {
                backend.register_model(Box::new(|data, _shape| data.to_vec()))
            })
            .unwrap();

        assert_ne!(id.as_u64(), 0);
        engine.shutdown();
    }

    #[test]
    fn test_oneshot_inference() {
        let mut engine =
            NeuralEngine::start_with(InferenceConfig::default(), test_backend_factory()).unwrap();

        let model_id = engine
            .register_model(|backend| {
                backend.register_model(Box::new(|data, _shape| data.to_vec()))
            })
            .unwrap();

        let (tx, rx) = crossbeam_channel::bounded(1);
        let request = TensorRequest {
            model_id,
            input: Arc::from([1.0_f32, 2.0, 3.0, 4.0].as_slice()),
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
        let mut engine =
            NeuralEngine::start_with(InferenceConfig::default(), test_backend_factory()).unwrap();

        let model_id = engine
            .register_model(|backend| {
                backend.register_model(Box::new(|data, _shape| data.to_vec()))
            })
            .unwrap();

        let (param_tx, param_rx) = crossbeam_channel::bounded::<ControlParams>(16);

        let buffer_size = 2;
        let request = TensorRequest {
            model_id,
            input: Arc::from([440.0_f32, 440.0, 0.5, 0.5].as_slice()),
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
        let mut engine =
            NeuralEngine::start_with(InferenceConfig::default(), test_backend_factory()).unwrap();

        let model_id = engine
            .register_model(|backend| {
                backend.register_model(Box::new(|data, _shape| {
                    data.iter().map(|x| x * 2.0).collect()
                }))
            })
            .unwrap();

        let queue = crate::gpu::shared_effect_queue(2, 2);

        queue.write_input(0, 0.1);
        queue.write_input(1, 0.2);
        queue.write_input(0, 0.3);
        queue.write_input(1, 0.4);

        let input_data = queue.take_input().unwrap();
        let features = input_data.len();

        let request = TensorRequest {
            model_id,
            input: Arc::from(input_data),
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
            input: Arc::from([1.0_f32].as_slice()),
            input_shape: [1, 1],
            response: ResponseChannel::OneShot(crossbeam_channel::bounded(1).0),
        };
        assert!(submit_request(&tx, request));
    }
}
