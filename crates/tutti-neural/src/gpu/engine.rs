//! Neural inference engine — pure tensor API.
//!
//! The engine is a model registry + forward pass executor.
//! It takes tensors in, runs model.forward(), returns tensors out.
//! It doesn't know about synths, effects, MIDI, or audio — just tensors.

use crate::error::Result;
use crate::gpu::fusion::NeuralModel;
use burn::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export NeuralModelId from tutti_core (canonical definition)
pub use tutti_core::neural::NeuralModelId;

/// Configuration for neural inference engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Batch size for processing multiple requests
    pub batch_size: usize,

    /// Enable INT8 quantization
    pub quantize: bool,

    /// Enable kernel fusion (CubeCL)
    pub enable_fusion: bool,

    /// Use graph-aware batching instead of timing-based.
    pub use_graph_aware_batching: bool,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            batch_size: 8,
            quantize: false,
            enable_fusion: true,
            use_graph_aware_batching: false,
        }
    }
}

/// Neural inference engine — pure tensor API.
///
/// Owns a set of models (closure-based) and runs forward passes.
/// Lives on a single dedicated inference thread, owned directly (no Arc).
/// Uses HashMap (not DashMap) since there's no concurrent access.
pub struct NeuralInferenceEngine<B: Backend> {
    config: InferenceConfig,
    device: B::Device,
    models: HashMap<NeuralModelId, NeuralModel<B>>,
    stats: InferenceStats,
}

/// Inference statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InferenceStats {
    pub total_inferences: u64,
    pub avg_latency_ms: f32,
    pub peak_latency_ms: f32,
    pub batch_hit_rate: f32,
}

impl<B: Backend> NeuralInferenceEngine<B> {
    /// Create a new engine. Must be called on the dedicated inference thread.
    pub fn new(device: B::Device, config: InferenceConfig) -> Result<Self> {
        Ok(Self {
            config,
            device,
            models: HashMap::new(),
            stats: InferenceStats::default(),
        })
    }

    /// Get the engine configuration.
    pub fn config(&self) -> &InferenceConfig {
        &self.config
    }

    /// Register a model. Returns its unique ID.
    pub fn register_model(&mut self, model: NeuralModel<B>) -> NeuralModelId {
        let id = NeuralModelId::new();
        self.models.insert(id, model);
        id
    }

    /// Run forward on multiple requests, grouping by model_id automatically.
    ///
    /// Each request is (model_id, flat_input_data, feature_dim).
    /// Returns output data in the same order as input.
    pub fn forward_grouped(
        &mut self,
        requests: &[(NeuralModelId, Vec<f32>, usize)],
    ) -> Result<Vec<Vec<f32>>> {
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

                let input = Tensor::<B, 1>::from_floats(all_data.as_slice(), &self.device)
                    .reshape([batch_size, first_dim]);
                let output = model.forward(input);
                let output_data = output.into_data();
                let all_output: Vec<f32> = output_data.to_vec::<f32>().expect("tensor to vec");

                let output_dim = all_output.len() / batch_size;
                for (i, (idx, _, _)) in batch.iter().enumerate() {
                    let s = i * output_dim;
                    results[*idx] = Some(all_output[s..s + output_dim].to_vec());
                }
            } else {
                for (idx, data, feat_dim) in batch {
                    let input =
                        Tensor::<B, 1>::from_floats(data, &self.device).reshape([1, feat_dim]);
                    let output = model.forward(input);
                    let output_data = output.into_data();
                    let result: Vec<f32> = output_data.to_vec::<f32>().expect("tensor to vec");
                    results[idx] = Some(result);
                }
            }
        }

        let latency_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.update_stats(latency_ms, requests.len() > 1);

        Ok(results.into_iter().map(|r| r.unwrap_or_default()).collect())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendPool;
    use burn::backend::ndarray::NdArray;

    #[test]
    fn test_engine_creation() {
        let device = burn::backend::ndarray::NdArrayDevice::default();
        let engine = NeuralInferenceEngine::<NdArray>::new(device, InferenceConfig::default());
        assert!(engine.is_ok());
    }

    #[test]
    fn test_register_and_forward_grouped() {
        let backend_pool = BackendPool::new().unwrap();
        let device = (**backend_pool.cpu_device()).clone();

        let mut engine =
            NeuralInferenceEngine::<NdArray>::new(device, InferenceConfig::default()).unwrap();

        let model_id = engine.register_model(
            crate::gpu::fusion::NeuralModel::<NdArray>::from_forward(|input| input),
        );

        let requests = vec![(model_id, vec![1.0, 2.0, 3.0], 3)];
        let results = engine.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn test_forward_grouped_batched() {
        let backend_pool = BackendPool::new().unwrap();
        let device = (**backend_pool.cpu_device()).clone();

        let mut engine =
            NeuralInferenceEngine::<NdArray>::new(device, InferenceConfig::default()).unwrap();

        let model_id = engine.register_model(
            crate::gpu::fusion::NeuralModel::<NdArray>::from_forward(|input| input),
        );

        let requests = vec![
            (model_id, vec![1.0, 2.0, 3.0, 4.0], 4),
            (model_id, vec![5.0, 6.0, 7.0, 8.0], 4),
        ];

        let results = engine.forward_grouped(&requests).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), 4);
        assert_eq!(results[1].len(), 4);
    }

    #[test]
    fn test_config_default() {
        let config = InferenceConfig::default();
        assert_eq!(config.batch_size, 8);
        assert!(!config.quantize);
        assert!(config.enable_fusion);
    }

    #[test]
    fn test_model_id_generation() {
        let id1 = NeuralModelId::new();
        let id2 = NeuralModelId::new();
        assert_ne!(id1.as_u64(), id2.as_u64());
    }

    #[test]
    fn test_inference_stats_default() {
        let stats = InferenceStats::default();
        assert_eq!(stats.total_inferences, 0);
        assert_eq!(stats.avg_latency_ms, 0.0);
        assert_eq!(stats.peak_latency_ms, 0.0);
        assert_eq!(stats.batch_hit_rate, 0.0);
    }
}
