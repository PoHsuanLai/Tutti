//! Batch processing for neural inference.

use std::time::Instant;

pub struct BatchCollector {
    max_batch_size: usize,
    max_wait_ms: u64,
    batch_start: Option<Instant>,
}

impl BatchCollector {
    pub fn new(max_batch_size: usize, max_wait_ms: u64) -> Self {
        Self {
            max_batch_size,
            max_wait_ms,
            batch_start: None,
        }
    }

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

    pub fn start_batch(&mut self) {
        self.batch_start = Some(Instant::now());
    }

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
        assert!(!collector.is_ready(4));
        assert!(collector.is_ready(8));
        assert!(collector.is_ready(10));
    }

    #[test]
    fn test_batch_collector_timeout() {
        let mut collector = BatchCollector::new(8, 50);

        collector.start_batch();
        assert!(!collector.is_ready(2));

        sleep(Duration::from_millis(60));
        assert!(collector.is_ready(2));

        collector.reset();
        assert!(!collector.is_ready(2));
    }
}
