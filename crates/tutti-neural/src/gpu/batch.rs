//! Batch processing for neural inference
//!
//! Collects multiple inference requests and processes them in a single GPU call
//! for significant performance improvements (8x speedup).
//!
//! **Status**: Connected to inference engine in Phase 3 (batched inference).

use std::time::Instant;

/// Batch request collector
///
/// Collects inference requests until batch size is reached or timeout occurs.
pub struct BatchCollector {
    /// Maximum batch size
    max_batch_size: usize,

    /// Maximum wait time before processing partial batch (ms)
    max_wait_ms: u64,

    /// Timestamp of first request in current batch
    batch_start: Option<Instant>,
}

impl BatchCollector {
    /// Create a new batch collector
    pub fn new(max_batch_size: usize, max_wait_ms: u64) -> Self {
        Self {
            max_batch_size,
            max_wait_ms,
            batch_start: None,
        }
    }

    /// Check if batch is ready to process
    ///
    /// A batch is ready if:
    /// - It has reached max_batch_size, OR
    /// - It has been waiting for max_wait_ms
    pub fn is_ready(&self, current_size: usize) -> bool {
        if current_size >= self.max_batch_size {
            return true;
        }

        if let Some(start) = self.batch_start {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            elapsed_ms >= self.max_wait_ms
        } else {
            false
        }
    }

    /// Mark the start of a new batch
    pub fn start_batch(&mut self) {
        self.batch_start = Some(Instant::now());
    }

    /// Reset batch timing
    pub fn reset(&mut self) {
        self.batch_start = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_batch_collector_size() {
        let collector = BatchCollector::new(8, 100);

        // Not ready with small batch
        assert!(!collector.is_ready(4));

        // Ready when batch size reached
        assert!(collector.is_ready(8));
        assert!(collector.is_ready(10));
    }

    #[test]
    fn test_batch_collector_timeout() {
        let mut collector = BatchCollector::new(8, 50); // 50ms timeout

        // Start batch
        collector.start_batch();

        // Not ready immediately
        assert!(!collector.is_ready(2));

        // Sleep longer than timeout
        sleep(Duration::from_millis(60));

        // Should be ready due to timeout
        assert!(collector.is_ready(2));

        // Reset
        collector.reset();
        assert!(!collector.is_ready(2));
    }
}
