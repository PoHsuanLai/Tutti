//! Burn ML backend for Tutti neural audio.
//!
//! Implements [`InferenceBackend`] using the [Burn](https://burn.dev) ML framework
//! with NdArray (CPU) and wgpu (GPU) backends. Models are automatically placed on
//! the GPU when available, with per-model CPU/GPU override via [`DevicePlacement`].
//!
//! # Usage
//!
//! ```rust,ignore
//! use tutti_burn::burn_backend_factory;
//!
//! // Create factory for the Burn backend
//! let factory = burn_backend_factory();
//!
//! // Pass to NeuralSystemBuilder
//! let neural = NeuralSystem::builder()
//!     .backend(factory)
//!     .build()?;
//! ```

mod backend_pool;
mod dispatch;
mod fusion;

pub use backend_pool::{BackendPool, CpuDevice, GpuBackendType, GpuInfo};
pub use dispatch::DevicePlacement;
pub use fusion::NeuralModel;

use burn::backend::wgpu::{Wgpu, WgpuDevice};
use burn::backend::NdArray;
use burn::prelude::*;
use dispatch::DeviceModel;
use std::collections::HashMap;
use tutti_core::neural::inference::{
    BackendCapabilities, BackendFactory, ForwardFn, InferenceBackend, InferenceConfig,
    InferenceError,
};
use tutti_core::NeuralModelId;

/// Inference statistics.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct InferenceStats {
    pub total_inferences: u64,
    pub avg_latency_ms: f32,
    pub peak_latency_ms: f32,
    pub batch_hit_rate: f32,
}

/// Burn-based inference backend with dynamic CPU/GPU dispatch.
///
/// Models can be placed on CPU (NdArray) or GPU (Wgpu + fusion) at registration
/// time. The default placement is GPU when available, falling back to CPU.
/// Operates on a single dedicated inference thread (Send but not Sync).
pub struct BurnInferenceBackend {
    config: InferenceConfig,
    cpu_device: <NdArray as Backend>::Device,
    gpu_device: Option<WgpuDevice>,
    models: HashMap<NeuralModelId, DeviceModel>,
    stats: InferenceStats,
    pool: BackendPool,
    default_placement: DevicePlacement,
}

impl BurnInferenceBackend {
    /// Create a new Burn inference backend.
    ///
    /// Automatically detects GPU availability. When a GPU is present, new models
    /// are placed there by default (override with [`set_default_placement`]).
    pub fn new(config: InferenceConfig) -> Result<Self, InferenceError> {
        let pool = BackendPool::new()?;
        let cpu_device = **pool.cpu_device();
        let gpu_device = pool.gpu_device().map(|d| (**d).clone());

        let default_placement = if gpu_device.is_some() {
            DevicePlacement::Gpu
        } else {
            DevicePlacement::Cpu
        };

        Ok(Self {
            config,
            cpu_device,
            gpu_device,
            models: HashMap::new(),
            stats: InferenceStats::default(),
            pool,
            default_placement,
        })
    }

    /// Set the default device placement for newly registered models.
    pub fn set_default_placement(&mut self, placement: DevicePlacement) {
        self.default_placement = placement;
    }

    /// Get the current default device placement.
    pub fn default_placement(&self) -> DevicePlacement {
        self.default_placement
    }

    /// Register a native Burn model on the CPU.
    pub fn register_cpu_model(&mut self, model: NeuralModel<NdArray>) -> NeuralModelId {
        let id = NeuralModelId::new();
        self.models.insert(
            id,
            DeviceModel::Cpu {
                model,
                device: self.cpu_device,
            },
        );
        id
    }

    /// Register a native Burn model on the GPU.
    ///
    /// Returns `Err` if no GPU is available.
    pub fn register_gpu_model(
        &mut self,
        model: NeuralModel<Wgpu>,
    ) -> Result<NeuralModelId, InferenceError> {
        let device = self.gpu_device.clone().ok_or_else(|| {
            InferenceError::BackendInit("Cannot register GPU model: no GPU available".into())
        })?;
        let id = NeuralModelId::new();
        self.models.insert(id, DeviceModel::Gpu { model, device });
        Ok(id)
    }

    /// Register a native Burn model on the CPU (backward-compatible alias).
    pub fn register_burn_model(&mut self, model: NeuralModel<NdArray>) -> NeuralModelId {
        self.register_cpu_model(model)
    }

    /// Query which device a model is placed on.
    pub fn model_placement(&self, id: &NeuralModelId) -> Option<DevicePlacement> {
        self.models.get(id).map(|m| m.placement())
    }

    /// Get inference statistics.
    pub fn stats(&self) -> &InferenceStats {
        &self.stats
    }

    /// Get the backend pool (for GPU info queries).
    pub fn pool(&self) -> &BackendPool {
        &self.pool
    }

    /// Resolve effective placement — falls back to CPU if GPU requested but unavailable.
    fn effective_placement(&self) -> DevicePlacement {
        match self.default_placement {
            DevicePlacement::Gpu if self.gpu_device.is_some() => DevicePlacement::Gpu,
            _ => DevicePlacement::Cpu,
        }
    }

    fn update_stats(&mut self, latency_ms: f32, batched: bool) {
        let stats = &mut self.stats;
        stats.total_inferences += 1;

        let alpha = 0.1;
        stats.avg_latency_ms = alpha * latency_ms + (1.0 - alpha) * stats.avg_latency_ms;

        if latency_ms > stats.peak_latency_ms {
            stats.peak_latency_ms = latency_ms;
        }

        if batched {
            stats.batch_hit_rate = alpha + (1.0 - alpha) * stats.batch_hit_rate;
        } else {
            stats.batch_hit_rate *= 1.0 - alpha;
        }
    }
}

impl InferenceBackend for BurnInferenceBackend {
    fn register_model(&mut self, f: ForwardFn) -> NeuralModelId {
        match self.effective_placement() {
            DevicePlacement::Cpu => {
                let device = self.cpu_device;
                let model =
                    NeuralModel::<NdArray>::from_forward(move |input: Tensor<NdArray, 2>| {
                        let shape = input.shape().dims;
                        let data: Vec<f32> =
                            input.into_data().to_vec::<f32>().expect("tensor to vec");
                        let result = f(&data, [shape[0], shape[1]]);
                        let len = result.len();
                        let batch = if shape[0] > 0 && len > 0 { shape[0] } else { 1 };
                        let features = if batch > 0 { len / batch } else { len };
                        Tensor::<NdArray, 1>::from_floats(result.as_slice(), &device)
                            .reshape([batch, features])
                    });
                self.register_cpu_model(model)
            }
            DevicePlacement::Gpu => {
                let device = self
                    .gpu_device
                    .clone()
                    .expect("GPU checked in effective_placement");
                let model = NeuralModel::<Wgpu>::from_forward(move |input: Tensor<Wgpu, 2>| {
                    let shape = input.shape().dims;
                    let data: Vec<f32> = input.into_data().to_vec::<f32>().expect("tensor to vec");
                    let result = f(&data, [shape[0], shape[1]]);
                    let len = result.len();
                    let batch = if shape[0] > 0 && len > 0 { shape[0] } else { 1 };
                    let features = if batch > 0 { len / batch } else { len };
                    Tensor::<Wgpu, 1>::from_floats(result.as_slice(), &device)
                        .reshape([batch, features])
                });
                let id = NeuralModelId::new();
                let device = self
                    .gpu_device
                    .clone()
                    .expect("GPU checked in effective_placement");
                self.models.insert(id, DeviceModel::Gpu { model, device });
                id
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn forward_grouped(
        &mut self,
        requests: &[(NeuralModelId, Vec<f32>, usize)],
    ) -> core::result::Result<Vec<Vec<f32>>, InferenceError> {
        if requests.is_empty() {
            return Ok(vec![]);
        }

        let start = std::time::Instant::now();

        // Group by model_id
        let mut grouped: HashMap<NeuralModelId, Vec<(usize, &[f32], usize)>> = HashMap::new();
        for (idx, (model_id, data, feat_dim)) in requests.iter().enumerate() {
            grouped
                .entry(*model_id)
                .or_default()
                .push((idx, data.as_slice(), *feat_dim));
        }

        let mut results: Vec<Option<Vec<f32>>> = vec![None; requests.len()];

        for (model_id, batch) in grouped {
            let model = match self.models.get(&model_id) {
                Some(m) => m,
                None => {
                    // Model not found — passthrough
                    for (idx, data, _) in &batch {
                        results[*idx] = Some(data.to_vec());
                    }
                    continue;
                }
            };

            let first_dim = batch[0].2;
            let can_batch = batch.len() > 1 && batch.iter().all(|(_, _, d)| *d == first_dim);

            if can_batch {
                let batch_size = batch.len();
                let mut all_data = Vec::with_capacity(batch_size * first_dim);
                for (_, data, _) in &batch {
                    all_data.extend_from_slice(data);
                }

                let all_output = model.forward_flat(&all_data, [batch_size, first_dim]);

                let output_dim = all_output.len() / batch_size;
                for (i, (idx, _, _)) in batch.iter().enumerate() {
                    let s = i * output_dim;
                    results[*idx] = Some(all_output[s..s + output_dim].to_vec());
                }
            } else {
                for (idx, data, feat_dim) in batch {
                    let output = model.forward_flat(data, [1, feat_dim]);
                    results[idx] = Some(output);
                }
            }
        }

        let latency_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.update_stats(latency_ms, requests.len() > 1);

        Ok(results.into_iter().map(|r| r.unwrap_or_default()).collect())
    }

    fn capabilities(&self) -> BackendCapabilities {
        let has_gpu_models = self
            .models
            .values()
            .any(|m| m.placement() == DevicePlacement::Gpu);
        let has_cpu_models = self
            .models
            .values()
            .any(|m| m.placement() == DevicePlacement::Cpu);

        let name = if has_gpu_models && has_cpu_models {
            "Burn/Hybrid(NdArray+wgpu)".into()
        } else if has_gpu_models {
            "Burn/wgpu".into()
        } else if has_cpu_models {
            "Burn/NdArray".into()
        } else if self.pool.has_gpu() {
            // No models registered yet — report wgpu since that's the default placement
            "Burn/wgpu".into()
        } else {
            "Burn/NdArray".into()
        };

        BackendCapabilities {
            name,
            supports_batching: true,
            has_gpu: self.pool.has_gpu(),
        }
    }

    fn config(&self) -> &InferenceConfig {
        &self.config
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }
}

/// Create a factory for the default Burn inference backend.
///
/// The returned factory, when called with an [`InferenceConfig`], initializes
/// a [`BurnInferenceBackend`] with CPU (NdArray) and optional GPU (wgpu + fusion)
/// support. Models are placed on GPU by default when available.
pub fn burn_backend_factory() -> BackendFactory {
    Box::new(|config| Ok(Box::new(BurnInferenceBackend::new(config)?) as Box<dyn InferenceBackend>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_burn_backend_creation() {
        let backend = BurnInferenceBackend::new(InferenceConfig::default());
        assert!(backend.is_ok());
    }

    #[test]
    fn test_register_and_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        // Force CPU for deterministic testing
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _shape| data.to_vec()));

        let requests = vec![(id, vec![1.0, 2.0, 3.0], 3)];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_register_burn_model() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();

        let id = backend.register_burn_model(NeuralModel::<NdArray>::from_forward(|input| input));

        let requests = vec![(id, vec![1.0, 2.0, 3.0, 4.0], 4)];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn test_batched_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _shape| data.to_vec()));

        let requests = vec![
            (id, vec![1.0, 2.0, 3.0, 4.0], 4),
            (id, vec![5.0, 6.0, 7.0, 8.0], 4),
        ];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 4);
        assert_eq!(results[1].len(), 4);
    }

    #[test]
    fn test_capabilities() {
        let backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        let caps = backend.capabilities();
        assert!(caps.supports_batching);
        assert!(caps.name.starts_with("Burn/"));
    }

    #[test]
    fn test_factory() {
        let factory = burn_backend_factory();
        let backend = factory(InferenceConfig::default());
        assert!(backend.is_ok());
    }

    #[test]
    fn test_default_placement_heuristic() {
        let backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        if backend.pool().has_gpu() {
            assert_eq!(backend.default_placement(), DevicePlacement::Gpu);
        } else {
            assert_eq!(backend.default_placement(), DevicePlacement::Cpu);
        }
    }

    #[test]
    fn test_set_default_placement() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);
        assert_eq!(backend.default_placement(), DevicePlacement::Cpu);
        backend.set_default_placement(DevicePlacement::Gpu);
        assert_eq!(backend.default_placement(), DevicePlacement::Gpu);
    }

    #[test]
    fn test_register_cpu_model_placement() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();

        let id = backend.register_cpu_model(NeuralModel::<NdArray>::from_forward(|input| input));

        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Cpu));
    }

    #[test]
    fn test_register_model_respects_default_placement() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _shape| data.to_vec()));
        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Cpu));
    }

    #[test]
    fn test_gpu_model_registration_and_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();

        if !backend.pool().has_gpu() {
            // No GPU — verify register_gpu_model errors
            let model = NeuralModel::<Wgpu>::from_forward(|input| input);
            assert!(backend.register_gpu_model(model).is_err());
            return;
        }

        // GPU available — register and run
        backend.set_default_placement(DevicePlacement::Gpu);

        let id = backend.register_model(Box::new(|data, _shape| data.to_vec()));
        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Gpu));

        let requests = vec![(id, vec![1.0, 2.0, 3.0], 3)];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_mixed_device_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();

        // Always register a CPU model
        let cpu_id =
            backend.register_cpu_model(NeuralModel::<NdArray>::from_forward(|input| input));

        if !backend.pool().has_gpu() {
            // No GPU — just test CPU model works
            let requests = vec![(cpu_id, vec![1.0, 2.0], 2)];
            let results = backend.forward_grouped(&requests).unwrap();
            assert_eq!(results[0], vec![1.0, 2.0]);
            return;
        }

        // GPU available — register GPU model and test both in one forward_grouped call
        backend.set_default_placement(DevicePlacement::Gpu);
        let gpu_id = backend.register_model(Box::new(|data, _shape| data.to_vec()));

        assert_eq!(backend.model_placement(&cpu_id), Some(DevicePlacement::Cpu));
        assert_eq!(backend.model_placement(&gpu_id), Some(DevicePlacement::Gpu));

        let requests = vec![
            (cpu_id, vec![1.0, 2.0], 2),
            (gpu_id, vec![3.0, 4.0], 2),
            (cpu_id, vec![5.0, 6.0], 2),
        ];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0], vec![1.0, 2.0]);
        assert_eq!(results[1], vec![3.0, 4.0]);
        assert_eq!(results[2], vec![5.0, 6.0]);
    }

    #[test]
    fn test_capabilities_reflects_models() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let _id = backend.register_model(Box::new(|data, _shape| data.to_vec()));

        // Only CPU models registered — capabilities should report NdArray
        let caps = backend.capabilities();
        assert_eq!(caps.name, "Burn/NdArray");
    }

    #[test]
    fn test_backward_compat_register_burn_model() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();

        // Old API should still work and place on CPU
        let id = backend.register_burn_model(NeuralModel::<NdArray>::from_forward(|input| input));
        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Cpu));

        let requests = vec![(id, vec![10.0, 20.0], 2)];
        let results = backend.forward_grouped(&requests).unwrap();
        assert_eq!(results[0], vec![10.0, 20.0]);
    }
}
