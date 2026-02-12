//! Lock-free ring buffers for audio streaming.

use ringbuf::{
    traits::{Consumer, Observer, Producer, Split},
    HeapCons, HeapProd, HeapRb,
};
use std::path::PathBuf;
use std::sync::Arc;
use tutti_core::{AtomicU64, Ordering};

use super::request::{CaptureId, RegionId};

/// Shared metadata for a region buffer.
#[derive(Debug)]
pub(crate) struct RegionBufferMeta {
    file_path: PathBuf,
    file_position: AtomicU64,
}

impl RegionBufferMeta {
    pub fn file_position(&self) -> u64 {
        self.file_position.load(Ordering::Relaxed)
    }

    pub fn set_file_position(&self, pos: u64) {
        self.file_position.store(pos, Ordering::Relaxed);
    }
}

/// Producer side of a region buffer owned by Butler thread.
pub(crate) struct RegionBufferProducer {
    prod: HeapProd<(f32, f32)>,
    meta: Arc<RegionBufferMeta>,
}

impl RegionBufferProducer {
    pub fn file_position(&self) -> u64 {
        self.meta.file_position()
    }

    pub fn set_file_position(&self, pos: u64) {
        self.meta.set_file_position(pos);
    }

    pub fn write_space(&self) -> usize {
        self.prod.vacant_len()
    }

    pub fn capacity(&self) -> usize {
        self.prod.capacity().get()
    }

    pub fn write(&mut self, samples: &[(f32, f32)]) -> usize {
        let mut written = 0;
        for &sample in samples {
            if self.prod.try_push(sample).is_ok() {
                written += 1;
            } else {
                break;
            }
        }
        written
    }

    /// Write samples in reverse order (for reverse playback).
    /// Samples are taken from the end of the slice first.
    pub fn write_reversed(&mut self, samples: &[(f32, f32)]) -> usize {
        let mut written = 0;
        for &sample in samples.iter().rev() {
            if self.prod.try_push(sample).is_ok() {
                written += 1;
            } else {
                break;
            }
        }
        written
    }

    pub fn file_path(&self) -> &PathBuf {
        &self.meta.file_path
    }
}

unsafe impl Send for RegionBufferProducer {}

/// Consumer side of a region buffer for audio callback.
pub struct RegionBufferConsumer {
    cons: HeapCons<(f32, f32)>,
    read_position: Arc<AtomicU64>,
    region_id: RegionId,
}

impl RegionBufferConsumer {
    pub(crate) fn region_id(&self) -> RegionId {
        self.region_id
    }

    /// Read the next sample from the buffer.
    /// Returns None if buffer is empty (underrun).
    #[inline]
    pub fn read(&mut self) -> Option<(f32, f32)> {
        self.cons.try_pop().inspect(|_| {
            self.read_position.fetch_add(1, Ordering::Relaxed);
        })
    }

    /// Clear all buffered samples without processing them.
    /// Used for loop resets — much faster than draining one-by-one.
    pub fn clear(&mut self) {
        let count = self.cons.occupied_len();
        for _ in 0..count {
            let _ = self.cons.try_pop();
        }
        self.read_position
            .fetch_add(count as u64, Ordering::Relaxed);
    }

    /// Get the number of samples available for reading.
    pub fn available(&self) -> usize {
        self.cons.occupied_len()
    }

    /// Get a shared handle to the read position for lock-free access.
    pub(crate) fn read_position_shared(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.read_position)
    }
}

unsafe impl Send for RegionBufferConsumer {}
unsafe impl Sync for RegionBufferConsumer {}

/// A region buffer for streaming audio.
pub(crate) struct RegionBuffer;

impl RegionBuffer {
    pub(crate) fn with_capacity(
        region_id: RegionId,
        file_path: PathBuf,
        capacity: usize,
    ) -> (RegionBufferProducer, RegionBufferConsumer) {
        let capacity = capacity.max(4096);

        let rb = HeapRb::<(f32, f32)>::new(capacity);
        let (prod, cons) = rb.split();

        let meta = Arc::new(RegionBufferMeta {
            file_path,
            file_position: AtomicU64::new(0),
        });

        let producer = RegionBufferProducer {
            prod,
            meta: meta.clone(),
        };

        let consumer = RegionBufferConsumer {
            cons,
            read_position: Arc::new(AtomicU64::new(0)),
            region_id,
        };

        (producer, consumer)
    }
}

/// Metadata for a capture buffer.
#[derive(Debug)]
pub(crate) struct CaptureBufferMeta {
    file_path: PathBuf,
    frames_written: AtomicU64,
    frames_captured: AtomicU64,
}

impl CaptureBufferMeta {
    fn add_frames_written(&self, count: u64) {
        self.frames_written.fetch_add(count, Ordering::Relaxed);
    }

    fn add_frames_captured(&self, count: u64) {
        self.frames_captured.fetch_add(count, Ordering::Relaxed);
    }
}

/// Producer side of capture buffer for audio callback writes.
pub struct CaptureBufferProducer {
    prod: HeapProd<(f32, f32)>,
    meta: Arc<CaptureBufferMeta>,
    capture_id: CaptureId,
}

unsafe impl Sync for CaptureBufferProducer {}

impl CaptureBufferProducer {
    pub fn capture_id(&self) -> CaptureId {
        self.capture_id
    }

    pub fn write_space(&self) -> usize {
        self.prod.vacant_len()
    }

    #[inline]
    pub fn write(&mut self, sample: (f32, f32)) -> bool {
        if self.prod.try_push(sample).is_ok() {
            self.meta.add_frames_captured(1);
            true
        } else {
            false
        }
    }

    pub fn write_many(&mut self, samples: &[(f32, f32)]) -> usize {
        let mut written = 0;
        for &sample in samples {
            if self.prod.try_push(sample).is_ok() {
                written += 1;
            } else {
                break;
            }
        }
        self.meta.add_frames_captured(written as u64);
        written
    }

    pub fn is_nearly_full(&self, threshold: usize) -> bool {
        self.write_space() < threshold
    }

    pub fn file_path(&self) -> &PathBuf {
        &self.meta.file_path
    }
}

/// Consumer side of capture buffer owned by Butler thread.
pub(crate) struct CaptureBufferConsumer {
    cons: HeapCons<(f32, f32)>,
    meta: Arc<CaptureBufferMeta>,
}

unsafe impl Send for CaptureBufferConsumer {}

impl CaptureBufferConsumer {
    pub(crate) fn available(&self) -> usize {
        self.cons.occupied_len()
    }

    pub(crate) fn read_into(&mut self, buffer: &mut [(f32, f32)]) -> usize {
        let mut read = 0;
        for slot in buffer.iter_mut() {
            if let Some(sample) = self.cons.try_pop() {
                *slot = sample;
                read += 1;
            } else {
                break;
            }
        }
        read
    }

    pub(crate) fn add_frames_written(&self, count: u64) {
        self.meta.add_frames_written(count);
    }
}

/// Capture buffer factory.
pub(crate) struct CaptureBuffer;

impl CaptureBuffer {
    #[allow(clippy::new_ret_no_self)]
    pub(crate) fn new(
        capture_id: CaptureId,
        file_path: PathBuf,
        sample_rate: f64,
        buffer_size_ms: f32,
    ) -> (CaptureBufferProducer, CaptureBufferConsumer) {
        let capacity = (buffer_size_ms / 1000.0 * sample_rate as f32) as usize;
        let capacity = capacity.max(4096);

        Self::with_capacity(capture_id, file_path, capacity)
    }

    pub(crate) fn with_capacity(
        capture_id: CaptureId,
        file_path: PathBuf,
        capacity: usize,
    ) -> (CaptureBufferProducer, CaptureBufferConsumer) {
        let capacity = capacity.max(4096);

        let rb = HeapRb::<(f32, f32)>::new(capacity);
        let (prod, cons) = rb.split();

        let meta = Arc::new(CaptureBufferMeta {
            file_path,
            frames_written: AtomicU64::new(0),
            frames_captured: AtomicU64::new(0),
        });

        let producer = CaptureBufferProducer {
            prod,
            meta: Arc::clone(&meta),
            capture_id,
        };

        let consumer = CaptureBufferConsumer { cons, meta };

        (producer, consumer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_region_buffer_creation() {
        let region_id = RegionId::generate();
        let capacity = (100.0 / 1000.0 * 44100.0) as usize; // 100ms buffer at 44.1kHz
        let (mut prod, mut cons) =
            RegionBuffer::with_capacity(region_id, PathBuf::from("test.wav"), capacity);

        // Write some samples
        let samples: Vec<_> = (0..100)
            .map(|i| (i as f32 / 100.0, i as f32 / 100.0))
            .collect();
        let written = prod.write(&samples);
        assert_eq!(written, 100);

        // Read them back
        let sample = cons.read().unwrap();
        assert_eq!(sample, (0.0, 0.0));
    }

    #[test]
    fn test_buffer_full() {
        let region_id = RegionId::generate();
        let (mut prod, _) = RegionBuffer::with_capacity(
            region_id,
            PathBuf::from("test.wav"),
            10, // Tiny buffer (will be clamped to 4096)
        );

        // Fill the buffer
        let samples: Vec<_> = (0..4096).map(|i| (i as f32, i as f32)).collect();
        let written = prod.write(&samples);
        assert!(written <= 4096);
    }

    #[test]
    fn test_clear_allows_refill() {
        let region_id = RegionId::generate();
        let capacity = 100;
        let (mut prod, mut cons) =
            RegionBuffer::with_capacity(region_id, PathBuf::from("test.wav"), capacity);

        // Fill buffer with "section A" data (values 0.0 - 0.99)
        let section_a: Vec<_> = (0..50).map(|i| (i as f32 / 100.0, i as f32 / 100.0)).collect();
        let written = prod.write(&section_a);
        assert_eq!(written, 50);

        // Verify we can read section A
        let first = cons.read().unwrap();
        assert!((first.0 - 0.0).abs() < 0.001, "First sample should be ~0.0");

        // Clear the buffer (simulating a seek)
        cons.clear();

        // After clear, write_space should be available for producer
        let write_space_after_clear = prod.write_space();
        assert!(
            write_space_after_clear > 0,
            "Producer should have write space after consumer clear, got {}",
            write_space_after_clear
        );

        // Write "section B" data (values 1.0 - 1.49)
        let section_b: Vec<_> = (0..50).map(|i| (1.0 + i as f32 / 100.0, 1.0 + i as f32 / 100.0)).collect();
        let written_b = prod.write(&section_b);
        assert!(written_b > 0, "Should be able to write after clear");

        // Read from consumer - should get section B data
        let sample_b = cons.read().unwrap();
        assert!(
            sample_b.0 >= 1.0,
            "After clear and refill, should read section B (>=1.0), got {}",
            sample_b.0
        );
    }

    /// Test full buffer clear and refill scenario (simulates seek)
    #[test]
    fn test_full_buffer_seek_simulation() {
        let region_id = RegionId::generate();
        // Use a larger buffer to match more realistic scenarios
        let capacity = 4096;
        let (mut prod, mut cons) =
            RegionBuffer::with_capacity(region_id, PathBuf::from("test.wav"), capacity);

        // Fill buffer completely with "220Hz-like" data (low values)
        let low_freq: Vec<_> = (0..4096).map(|i| {
            let phase = (i as f32 * 0.03).sin(); // ~220Hz pattern
            (phase, phase)
        }).collect();
        let written_low = prod.write(&low_freq);
        eprintln!("Wrote {} low-freq samples", written_low);

        // Read a few samples to simulate audio playback
        for _ in 0..100 {
            let _ = cons.read();
        }

        let write_space_before = prod.write_space();
        eprintln!("Write space before clear: {}", write_space_before);

        // Clear the buffer (simulating seek)
        cons.clear();

        let write_space_after = prod.write_space();
        eprintln!("Write space after clear: {}", write_space_after);

        // The key assertion: producer should see MORE write space after clear
        assert!(
            write_space_after > write_space_before,
            "Producer write space should increase after clear: before={}, after={}",
            write_space_before,
            write_space_after
        );

        // Refill with "880Hz-like" data (high values)
        let high_freq: Vec<_> = (0..write_space_after).map(|i| {
            let phase = (i as f32 * 0.125).sin(); // ~880Hz pattern
            (phase, phase)
        }).collect();
        let written_high = prod.write(&high_freq);
        eprintln!("Wrote {} high-freq samples after clear", written_high);

        // Read should get high-freq data, not low-freq
        let sample = cons.read().unwrap();

        // First sample of high-freq (i=0): sin(0) = 0.0
        // Second sample: sin(0.125) ≈ 0.125
        // Compare to low-freq first sample: sin(0) = 0.0, second: sin(0.03) ≈ 0.03

        let sample2 = cons.read().unwrap();
        let sample3 = cons.read().unwrap();
        let sample_diff = sample3.0 - sample2.0;

        eprintln!("Sample values: {:?}, {:?}, {:?}", sample, sample2, sample3);
        eprintln!("Diff between samples: {}", sample_diff);

        // High freq should have larger differences between samples
        // Low freq: sin(0.03) - sin(0) ≈ 0.03
        // High freq: sin(0.25) - sin(0.125) ≈ 0.12
        assert!(
            sample_diff.abs() > 0.05,
            "After refill, should get high-freq data with larger sample diff, got {}",
            sample_diff.abs()
        );
    }
}
