//! Graph-aware neural inference batching.

use super::engine::{InferenceRequest, InferenceResponse, NeuralInferenceEngine};
use burn::prelude::Backend;
use std::collections::HashMap;
use std::sync::Arc;

pub use tutti_core::neural::BatchingStrategy;

/// Graph-aware batcher for neural inference requests.
pub struct GraphAwareBatcher<B: Backend> {
    engine: Arc<NeuralInferenceEngine<B>>,
    current_strategy: Option<BatchingStrategy>,
    pending_requests: HashMap<u64, Vec<InferenceRequest>>,
    stats: BatcherStats,
}

impl<B: Backend> GraphAwareBatcher<B> {
    pub fn new(engine: Arc<NeuralInferenceEngine<B>>) -> Self {
        Self {
            engine,
            current_strategy: None,
            pending_requests: HashMap::new(),
            stats: BatcherStats::default(),
        }
    }

    pub fn set_strategy(&mut self, strategy: BatchingStrategy) {
        self.current_strategy = Some(strategy);
    }

    pub fn strategy(&self) -> Option<&BatchingStrategy> {
        self.current_strategy.as_ref()
    }

    pub fn queue_request(&mut self, request: InferenceRequest) {
        let model_key = request.model_id.as_u64();
        self.pending_requests
            .entry(model_key)
            .or_default()
            .push(request);
        self.stats.requests_queued += 1;
    }

    /// Check if there are pending requests
    pub fn has_pending(&self) -> bool {
        self.pending_requests.values().any(|v| !v.is_empty())
    }

    /// Get count of pending requests
    #[allow(dead_code)]
    pub fn pending_count(&self) -> usize {
        self.pending_requests.values().map(|v| v.len()).sum()
    }

    /// Process all pending requests using graph-aware batching
    ///
    /// Returns responses for all processed requests.
    pub fn process_batches(&mut self) -> crate::error::Result<Vec<InferenceResponse>> {
        let strategy = match &self.current_strategy {
            Some(s) => s,
            None => {
                // No strategy yet - process requests individually by model
                return self.process_without_strategy();
            }
        };

        if self.pending_requests.is_empty() {
            return Ok(vec![]);
        }

        let mut all_responses = Vec::new();
        let start = std::time::Instant::now();

        // Process each model's batch according to strategy
        for model_id in strategy.model_batches.keys() {
            let model_key = model_id.as_u64();
            if let Some(requests) = self.pending_requests.remove(&model_key) {
                if requests.is_empty() {
                    continue;
                }

                let batch_size = requests.len();
                let responses = self.engine.infer_batch(&requests)?;

                self.stats.batches_processed += 1;
                self.stats.requests_processed += batch_size;

                all_responses.extend(responses);
            }
        }

        // Handle any remaining requests (models not in strategy)
        let remaining_models: Vec<_> = self.pending_requests.keys().cloned().collect();
        for model_key in remaining_models {
            if let Some(requests) = self.pending_requests.remove(&model_key) {
                if requests.is_empty() {
                    continue;
                }

                let batch_size = requests.len();
                let responses = self.engine.infer_batch(&requests)?;

                self.stats.batches_processed += 1;
                self.stats.requests_processed += batch_size;

                all_responses.extend(responses);
            }
        }

        let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.stats.total_latency_ms += elapsed_ms;

        Ok(all_responses)
    }

    /// Process requests without a batching strategy
    ///
    /// Falls back to grouping by model_id only.
    fn process_without_strategy(&mut self) -> crate::error::Result<Vec<InferenceResponse>> {
        if self.pending_requests.is_empty() {
            return Ok(vec![]);
        }

        let mut all_responses = Vec::new();
        let start = std::time::Instant::now();

        // Process each model's requests as a batch
        let models: Vec<_> = self.pending_requests.keys().cloned().collect();
        for model_id in models {
            if let Some(requests) = self.pending_requests.remove(&model_id) {
                if requests.is_empty() {
                    continue;
                }

                let batch_size = requests.len();
                let responses = self.engine.infer_batch(&requests)?;

                self.stats.batches_processed += 1;
                self.stats.requests_processed += batch_size;

                all_responses.extend(responses);
            }
        }

        let elapsed_ms = start.elapsed().as_secs_f32() * 1000.0;
        self.stats.total_latency_ms += elapsed_ms;

        Ok(all_responses)
    }

    /// Get batcher statistics
    pub fn stats(&self) -> &BatcherStats {
        &self.stats
    }
}

/// Statistics for the graph-aware batcher
#[derive(Debug, Clone, Default)]
pub struct BatcherStats {
    /// Total requests queued
    pub requests_queued: usize,

    /// Total requests processed
    pub requests_processed: usize,

    /// Total batches processed
    pub batches_processed: usize,

    /// Total processing latency (ms)
    pub total_latency_ms: f32,
}

impl BatcherStats {
    /// Get average batch size
    pub fn avg_batch_size(&self) -> f32 {
        if self.batches_processed == 0 {
            0.0
        } else {
            self.requests_processed as f32 / self.batches_processed as f32
        }
    }

    /// Get average latency per request (ms)
    pub fn avg_latency_per_request(&self) -> f32 {
        if self.requests_processed == 0 {
            0.0
        } else {
            self.total_latency_ms / self.requests_processed as f32
        }
    }

    /// Get average latency per batch (ms)
    pub fn avg_latency_per_batch(&self) -> f32 {
        if self.batches_processed == 0 {
            0.0
        } else {
            self.total_latency_ms / self.batches_processed as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use burn::backend::NdArray;

    type TestBackend = NdArray;
    type TestDevice = burn::backend::ndarray::NdArrayDevice;

    fn test_device() -> Arc<TestDevice> {
        Arc::new(TestDevice::default())
    }

    #[test]
    fn test_batcher_creation() {
        let device = test_device();
        let config = super::super::engine::InferenceConfig::default();
        let engine: NeuralInferenceEngine<TestBackend> =
            NeuralInferenceEngine::new(device, config).unwrap();

        #[allow(clippy::arc_with_non_send_sync)] // NdArray is single-threaded by design
        let batcher = GraphAwareBatcher::new(Arc::new(engine));

        assert!(!batcher.has_pending());
        assert_eq!(batcher.pending_count(), 0);
        assert!(batcher.strategy().is_none());
    }

    #[test]
    fn test_stats() {
        let stats = BatcherStats {
            requests_queued: 10,
            requests_processed: 8,
            batches_processed: 2,
            total_latency_ms: 10.0,
        };

        assert_eq!(stats.avg_batch_size(), 4.0);
        assert_eq!(stats.avg_latency_per_request(), 1.25);
        assert_eq!(stats.avg_latency_per_batch(), 5.0);
    }
}
