//! Burn ML backend for Tutti neural audio.
//!
//! Provides [`InferenceBackend`] using [Burn](https://burn.dev) with NdArray (CPU)
//! and wgpu (GPU) backends. Models are placed on GPU when available.
//!
//! ```rust,ignore
//! let neural = NeuralSystem::builder()
//!     .backend(tutti_burn::burn_backend_factory())
//!     .build()?;
//! ```

mod backend_pool;
mod dispatch;
mod fusion;

pub use dispatch::DevicePlacement;

use backend_pool::BackendPool;
use burn::backend::wgpu::{Wgpu, WgpuDevice};
use burn::backend::NdArray;
use burn::prelude::*;
use dispatch::DeviceModel;
use fusion::NeuralModel;
use std::collections::HashMap;
use tutti_core::neural::inference::{
    BackendCapabilities, BackendFactory, ForwardFn, InferenceBackend, InferenceConfig,
    InferenceError,
};
use tutti_core::NeuralModelId;

/// Burn-based inference backend with CPU/GPU dispatch.
pub struct BurnInferenceBackend {
    config: InferenceConfig,
    cpu_device: <NdArray as Backend>::Device,
    gpu_device: Option<WgpuDevice>,
    models: HashMap<NeuralModelId, DeviceModel>,
    pool: BackendPool,
    default_placement: DevicePlacement,
}

impl BurnInferenceBackend {
    /// Create backend with automatic GPU detection.
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
            pool,
            default_placement,
        })
    }

    pub fn set_default_placement(&mut self, placement: DevicePlacement) {
        self.default_placement = placement;
    }

    pub fn default_placement(&self) -> DevicePlacement {
        self.default_placement
    }

    pub fn model_placement(&self, id: &NeuralModelId) -> Option<DevicePlacement> {
        self.models.get(id).map(|m| m.placement())
    }

    fn effective_placement(&self) -> DevicePlacement {
        match self.default_placement {
            DevicePlacement::Gpu if self.gpu_device.is_some() => DevicePlacement::Gpu,
            _ => DevicePlacement::Cpu,
        }
    }
}

impl InferenceBackend for BurnInferenceBackend {
    fn register_model(&mut self, f: ForwardFn) -> NeuralModelId {
        let id = NeuralModelId::new();

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
                self.models.insert(
                    id,
                    DeviceModel::Cpu {
                        model,
                        device: self.cpu_device,
                    },
                );
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
                let device = self
                    .gpu_device
                    .clone()
                    .expect("GPU checked in effective_placement");
                self.models.insert(id, DeviceModel::Gpu { model, device });
            }
        }

        id
    }

    #[allow(clippy::type_complexity)]
    fn forward_grouped(
        &mut self,
        requests: &[(NeuralModelId, Vec<f32>, usize)],
    ) -> core::result::Result<Vec<Vec<f32>>, InferenceError> {
        if requests.is_empty() {
            return Ok(vec![]);
        }

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

/// Factory for the default Burn inference backend.
pub fn burn_backend_factory() -> BackendFactory {
    Box::new(|config| Ok(Box::new(BurnInferenceBackend::new(config)?) as Box<dyn InferenceBackend>))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_creation() {
        assert!(BurnInferenceBackend::new(InferenceConfig::default()).is_ok());
    }

    #[test]
    fn test_register_and_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _| data.to_vec()));
        let results = backend
            .forward_grouped(&[(id, vec![1.0, 2.0, 3.0], 3)])
            .unwrap();

        assert_eq!(results, vec![vec![1.0, 2.0, 3.0]]);
    }

    #[test]
    fn test_batched_forward() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _| data.to_vec()));
        let results = backend
            .forward_grouped(&[
                (id, vec![1.0, 2.0, 3.0, 4.0], 4),
                (id, vec![5.0, 6.0, 7.0, 8.0], 4),
            ])
            .unwrap();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 4);
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
        assert!(burn_backend_factory()(InferenceConfig::default()).is_ok());
    }

    #[test]
    fn test_default_placement_heuristic() {
        let backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        let expected = if backend.capabilities().has_gpu {
            DevicePlacement::Gpu
        } else {
            DevicePlacement::Cpu
        };
        assert_eq!(backend.default_placement(), expected);
    }

    #[test]
    fn test_placement_override() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        backend.set_default_placement(DevicePlacement::Cpu);

        let id = backend.register_model(Box::new(|data, _| data.to_vec()));
        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Cpu));
    }

    #[test]
    fn test_gpu_forward_if_available() {
        let mut backend = BurnInferenceBackend::new(InferenceConfig::default()).unwrap();
        if !backend.capabilities().has_gpu {
            return;
        }

        backend.set_default_placement(DevicePlacement::Gpu);
        let id = backend.register_model(Box::new(|data, _| data.to_vec()));

        assert_eq!(backend.model_placement(&id), Some(DevicePlacement::Gpu));
        let results = backend
            .forward_grouped(&[(id, vec![1.0, 2.0, 3.0], 3)])
            .unwrap();
        assert_eq!(results[0], vec![1.0, 2.0, 3.0]);
    }
}
