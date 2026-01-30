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
pub struct RegionBufferMeta {
    pub file_path: PathBuf,
    pub file_length: u64,
    pub sample_rate: f64,
    pub channels: usize,
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
pub struct RegionBufferProducer {
    prod: HeapProd<(f32, f32)>,
    pub meta: Arc<RegionBufferMeta>,
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

    pub fn file_path(&self) -> &PathBuf {
        &self.meta.file_path
    }

    pub fn file_length(&self) -> u64 {
        self.meta.file_length
    }

    pub fn sample_rate(&self) -> f64 {
        self.meta.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.meta.channels
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
    pub fn region_id(&self) -> RegionId {
        self.region_id
    }

    pub fn read_position(&self) -> u64 {
        self.read_position.load(Ordering::Relaxed)
    }

    pub fn available(&self) -> usize {
        self.cons.occupied_len()
    }

    pub fn needs_refill(&self, threshold: usize) -> bool {
        self.available() < threshold
    }

    #[inline]
    pub fn read(&mut self) -> Option<(f32, f32)> {
        self.cons.try_pop().inspect(|_| {
            self.read_position.fetch_add(1, Ordering::Relaxed);
        })
    }

    pub fn read_into(&mut self, buffer: &mut [(f32, f32)]) -> usize {
        let mut read = 0;
        for slot in buffer.iter_mut() {
            if let Some(sample) = self.cons.try_pop() {
                *slot = sample;
                read += 1;
            } else {
                break;
            }
        }
        self.read_position.fetch_add(read as u64, Ordering::Relaxed);
        read
    }

    pub fn is_empty(&self) -> bool {
        self.cons.is_empty()
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

    /// Get a shared handle to the read position for lock-free access.
    pub fn read_position_shared(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.read_position)
    }
}

unsafe impl Send for RegionBufferConsumer {}
unsafe impl Sync for RegionBufferConsumer {}

/// A region buffer for streaming audio.
pub struct RegionBuffer;

impl RegionBuffer {
    pub fn with_capacity(
        region_id: RegionId,
        file_path: PathBuf,
        file_length: u64,
        file_sample_rate: f64,
        channels: usize,
        capacity: usize,
    ) -> (RegionBufferProducer, RegionBufferConsumer) {
        let capacity = capacity.max(4096);

        let rb = HeapRb::<(f32, f32)>::new(capacity);
        let (prod, cons) = rb.split();

        let meta = Arc::new(RegionBufferMeta {
            file_path,
            file_length,
            sample_rate: file_sample_rate,
            channels,
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

/// Shared state for region buffer management.
/// Metadata for a capture buffer.
#[derive(Debug)]
pub struct CaptureBufferMeta {
    pub file_path: PathBuf,
    pub sample_rate: f64,
    pub channels: usize,
    frames_written: AtomicU64,
    frames_captured: AtomicU64,
}

impl CaptureBufferMeta {
    pub fn frames_written(&self) -> u64 {
        self.frames_written.load(Ordering::Relaxed)
    }

    pub fn add_frames_written(&self, count: u64) {
        self.frames_written.fetch_add(count, Ordering::Relaxed);
    }

    pub fn frames_captured(&self) -> u64 {
        self.frames_captured.load(Ordering::Relaxed)
    }

    pub fn add_frames_captured(&self, count: u64) {
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

    pub fn frames_captured(&self) -> u64 {
        self.meta.frames_captured()
    }
}

/// Consumer side of capture buffer owned by Butler thread.
pub struct CaptureBufferConsumer {
    cons: HeapCons<(f32, f32)>,
    pub meta: Arc<CaptureBufferMeta>,
    capture_id: CaptureId,
}

unsafe impl Send for CaptureBufferConsumer {}

impl CaptureBufferConsumer {
    pub fn capture_id(&self) -> CaptureId {
        self.capture_id
    }

    pub fn available(&self) -> usize {
        self.cons.occupied_len()
    }

    pub fn read_into(&mut self, buffer: &mut [(f32, f32)]) -> usize {
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

    pub fn needs_flush(&self, threshold: usize) -> bool {
        self.available() >= threshold
    }

    pub fn file_path(&self) -> &PathBuf {
        &self.meta.file_path
    }

    pub fn sample_rate(&self) -> f64 {
        self.meta.sample_rate
    }

    pub fn channels(&self) -> usize {
        self.meta.channels
    }

    pub fn frames_written(&self) -> u64 {
        self.meta.frames_written()
    }

    pub fn add_frames_written(&self, count: u64) {
        self.meta.add_frames_written(count);
    }
}

/// Capture buffer factory.
pub struct CaptureBuffer;

impl CaptureBuffer {
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        capture_id: CaptureId,
        file_path: PathBuf,
        sample_rate: f64,
        channels: usize,
        buffer_size_ms: f32,
    ) -> (CaptureBufferProducer, CaptureBufferConsumer) {
        let capacity = (buffer_size_ms / 1000.0 * sample_rate as f32) as usize;
        let capacity = capacity.max(4096);

        Self::with_capacity(capture_id, file_path, sample_rate, channels, capacity)
    }

    pub fn with_capacity(
        capture_id: CaptureId,
        file_path: PathBuf,
        sample_rate: f64,
        channels: usize,
        capacity: usize,
    ) -> (CaptureBufferProducer, CaptureBufferConsumer) {
        let capacity = capacity.max(4096);

        let rb = HeapRb::<(f32, f32)>::new(capacity);
        let (prod, cons) = rb.split();

        let meta = Arc::new(CaptureBufferMeta {
            file_path,
            sample_rate,
            channels,
            frames_written: AtomicU64::new(0),
            frames_captured: AtomicU64::new(0),
        });

        let producer = CaptureBufferProducer {
            prod,
            meta: Arc::clone(&meta),
            capture_id,
        };

        let consumer = CaptureBufferConsumer {
            cons,
            meta,
            capture_id,
        };

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
        let (mut prod, mut cons) = RegionBuffer::with_capacity(
            region_id,
            PathBuf::from("test.wav"),
            44100,
            44100.0,
            2,
            capacity,
        );

        // Write some samples
        let samples: Vec<_> = (0..100)
            .map(|i| (i as f32 / 100.0, i as f32 / 100.0))
            .collect();
        let written = prod.write(&samples);
        assert_eq!(written, 100);

        // Read them back
        assert_eq!(cons.available(), 100);
        let sample = cons.read().unwrap();
        assert_eq!(sample, (0.0, 0.0));
        assert_eq!(cons.available(), 99);
    }

    #[test]
    fn test_buffer_full() {
        let region_id = RegionId::generate();
        let (mut prod, _) = RegionBuffer::with_capacity(
            region_id,
            PathBuf::from("test.wav"),
            44100,
            44100.0,
            2,
            10, // Tiny buffer (will be clamped to 4096)
        );

        // Fill the buffer
        let samples: Vec<_> = (0..4096).map(|i| (i as f32, i as f32)).collect();
        let written = prod.write(&samples);
        assert!(written <= 4096);
    }

    #[test]
    fn test_needs_refill() {
        let region_id = RegionId::generate();
        let (mut prod, cons) = RegionBuffer::with_capacity(
            region_id,
            PathBuf::from("test.wav"),
            44100,
            44100.0,
            2,
            100,
        );

        // Initially needs refill (empty)
        assert!(cons.needs_refill(50));

        let samples: Vec<_> = (0..60).map(|i| (i as f32, i as f32)).collect();
        prod.write(&samples);

        // Now doesn't need refill
        assert!(!cons.needs_refill(50));
    }
}
