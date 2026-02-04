//! I/O statistics and metrics for butler thread.
//!
//! Tracks throughput, cache efficiency, and buffer health.

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// I/O metrics for butler thread operations.
pub struct IOMetrics {
    /// Bytes read from disk
    bytes_read: AtomicU64,
    /// Bytes written to disk (recording)
    bytes_written: AtomicU64,
    /// Read operations count
    read_ops: AtomicU64,
    /// Write operations count
    write_ops: AtomicU64,
    /// Cache hits
    cache_hits: AtomicU64,
    /// Cache misses
    cache_misses: AtomicU64,
    /// Low buffer events (<10% fill)
    low_buffer_events: AtomicU64,
    /// Throughput tracking for varifill (protected by mutex, only accessed from butler thread)
    throughput: Mutex<ThroughputTracker>,
}

impl Default for IOMetrics {
    fn default() -> Self {
        Self {
            bytes_read: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            read_ops: AtomicU64::new(0),
            write_ops: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            low_buffer_events: AtomicU64::new(0),
            throughput: Mutex::new(ThroughputTracker::new()),
        }
    }
}

/// Tracks recent I/O throughput using a sliding window.
struct ThroughputTracker {
    /// Recent read operations (bytes, timestamp)
    recent_reads: Vec<(u64, Instant)>,
    /// Window duration for throughput calculation (1 second)
    window_secs: f64,
    /// Cached read rate (bytes/sec), updated on each record
    cached_read_rate: f64,
}

impl ThroughputTracker {
    fn new() -> Self {
        Self {
            recent_reads: Vec::with_capacity(64),
            window_secs: 1.0,
            cached_read_rate: 0.0,
        }
    }

    fn record_read(&mut self, bytes: u64) {
        let now = Instant::now();
        self.recent_reads.push((bytes, now));

        self.update_rate(now);
    }

    fn update_rate(&mut self, now: Instant) {
        let cutoff = now - std::time::Duration::from_secs_f64(self.window_secs);
        self.recent_reads.retain(|(_, ts)| *ts > cutoff);

        if self.recent_reads.is_empty() {
            self.cached_read_rate = 0.0;
        } else {
            let total_bytes: u64 = self.recent_reads.iter().map(|(b, _)| *b).sum();
            if let (Some(first), Some(last)) = (self.recent_reads.first(), self.recent_reads.last())
            {
                let duration = last.1.duration_since(first.1).as_secs_f64();
                if duration > 0.01 {
                    self.cached_read_rate = total_bytes as f64 / duration;
                } else {
                    self.cached_read_rate = total_bytes as f64 / self.window_secs;
                }
            } else {
                self.cached_read_rate = total_bytes as f64 / self.window_secs;
            }
        }
    }

    fn read_rate(&self) -> f64 {
        self.cached_read_rate
    }
}

impl IOMetrics {
    /// Create new metrics tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record bytes read from disk.
    #[inline]
    pub fn record_read(&self, bytes: u64) {
        self.bytes_read.fetch_add(bytes, Ordering::Relaxed);
        self.read_ops.fetch_add(1, Ordering::Relaxed);
        if let Some(mut tracker) = self.throughput.try_lock() {
            tracker.record_read(bytes);
        }
    }

    /// Get recent read throughput in bytes/second.
    /// Used by varifill strategy to adapt chunk sizes.
    pub fn read_rate(&self) -> f64 {
        self.throughput.lock().read_rate()
    }

    /// Record bytes written to disk.
    #[inline]
    pub fn record_write(&self, bytes: u64) {
        self.bytes_written.fetch_add(bytes, Ordering::Relaxed);
        self.write_ops.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache hit.
    #[inline]
    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a cache miss.
    #[inline]
    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    /// Record a low buffer event.
    #[inline]
    pub fn record_low_buffer(&self) {
        self.low_buffer_events.fetch_add(1, Ordering::Relaxed);
    }

    /// Take a snapshot of current metrics.
    pub fn snapshot(&self) -> IOMetricsSnapshot {
        IOMetricsSnapshot {
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            bytes_written: self.bytes_written.load(Ordering::Relaxed),
            read_ops: self.read_ops.load(Ordering::Relaxed),
            write_ops: self.write_ops.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            low_buffer_events: self.low_buffer_events.load(Ordering::Relaxed),
            read_rate: self.read_rate(),
        }
    }

    /// Reset all metrics to zero.
    pub fn reset(&self) {
        self.bytes_read.store(0, Ordering::Relaxed);
        self.bytes_written.store(0, Ordering::Relaxed);
        self.read_ops.store(0, Ordering::Relaxed);
        self.write_ops.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.low_buffer_events.store(0, Ordering::Relaxed);
    }
}

/// Snapshot of I/O metrics at a point in time.
#[derive(Debug, Clone)]
pub struct IOMetricsSnapshot {
    /// Bytes read from disk
    pub bytes_read: u64,
    /// Bytes written to disk (recording)
    pub bytes_written: u64,
    /// Read operations count
    pub read_ops: u64,
    /// Write operations count
    pub write_ops: u64,
    /// Cache hits
    pub cache_hits: u64,
    /// Cache misses
    pub cache_misses: u64,
    /// Low buffer events (<10% fill)
    pub low_buffer_events: u64,
    /// Recent read throughput (bytes/second)
    pub read_rate: f64,
}

impl Default for IOMetricsSnapshot {
    fn default() -> Self {
        Self {
            bytes_read: 0,
            bytes_written: 0,
            read_ops: 0,
            write_ops: 0,
            cache_hits: 0,
            cache_misses: 0,
            low_buffer_events: 0,
            read_rate: 0.0,
        }
    }
}

impl IOMetricsSnapshot {
    /// Calculate cache hit rate (0.0 - 1.0).
    ///
    /// Returns 1.0 if no cache operations have occurred.
    pub fn cache_hit_rate(&self) -> f32 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            1.0
        } else {
            self.cache_hits as f32 / total as f32
        }
    }

    /// Calculate average bytes per read operation.
    pub fn avg_read_size(&self) -> u64 {
        if self.read_ops == 0 {
            0
        } else {
            self.bytes_read / self.read_ops
        }
    }

    /// Calculate average bytes per write operation.
    pub fn avg_write_size(&self) -> u64 {
        if self.write_ops == 0 {
            0
        } else {
            self.bytes_written / self.write_ops
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_recording() {
        let metrics = IOMetrics::new();

        metrics.record_read(1024);
        metrics.record_read(2048);
        metrics.record_write(512);
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();
        metrics.record_low_buffer();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.bytes_read, 3072);
        assert_eq!(snapshot.bytes_written, 512);
        assert_eq!(snapshot.read_ops, 2);
        assert_eq!(snapshot.write_ops, 1);
        assert_eq!(snapshot.cache_hits, 2);
        assert_eq!(snapshot.cache_misses, 1);
        assert_eq!(snapshot.low_buffer_events, 1);
    }

    #[test]
    fn test_cache_hit_rate() {
        let snapshot = IOMetricsSnapshot {
            cache_hits: 75,
            cache_misses: 25,
            ..Default::default()
        };
        assert!((snapshot.cache_hit_rate() - 0.75).abs() < 0.001);

        let empty = IOMetricsSnapshot::default();
        assert!((empty.cache_hit_rate() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_metrics_reset() {
        let metrics = IOMetrics::new();
        metrics.record_read(1024);
        metrics.record_cache_hit();

        let before = metrics.snapshot();
        assert_eq!(before.bytes_read, 1024);
        assert_eq!(before.cache_hits, 1);

        metrics.reset();

        let after = metrics.snapshot();
        assert_eq!(after.bytes_read, 0);
        assert_eq!(after.cache_hits, 0);
    }

    #[test]
    fn test_avg_sizes() {
        let snapshot = IOMetricsSnapshot {
            bytes_read: 10000,
            read_ops: 10,
            bytes_written: 5000,
            write_ops: 5,
            ..Default::default()
        };
        assert_eq!(snapshot.avg_read_size(), 1000);
        assert_eq!(snapshot.avg_write_size(), 1000);

        let empty = IOMetricsSnapshot::default();
        assert_eq!(empty.avg_read_size(), 0);
        assert_eq!(empty.avg_write_size(), 0);
    }
}
