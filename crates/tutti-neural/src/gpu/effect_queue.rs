//! Lock-free audio buffers for neural effect processing.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// SPSC lock-free double-buffered audio queue for effect processing.
/// Audio thread writes input / reads output; inference thread reads input / writes output.
pub struct EffectAudioQueue {
    input_buffers: [UnsafeCell<Vec<f32>>; 2],
    output_buffers: [UnsafeCell<Vec<f32>>; 2],
    input_write_idx: AtomicUsize,
    output_read_idx: AtomicUsize,
    input_ready: AtomicBool,
    output_ready: AtomicBool,
    channels: usize,
    buffer_size: usize,
    input_write_pos: AtomicUsize,
    output_read_pos: AtomicUsize,
}

impl EffectAudioQueue {
    pub fn new(channels: usize, buffer_size: usize) -> Self {
        let total_samples = channels * buffer_size;
        Self {
            input_buffers: [
                UnsafeCell::new(vec![0.0; total_samples]),
                UnsafeCell::new(vec![0.0; total_samples]),
            ],
            output_buffers: [
                UnsafeCell::new(vec![0.0; total_samples]),
                UnsafeCell::new(vec![0.0; total_samples]),
            ],
            input_write_idx: AtomicUsize::new(0),
            output_read_idx: AtomicUsize::new(0),
            input_ready: AtomicBool::new(false),
            output_ready: AtomicBool::new(false),
            channels,
            buffer_size,
            input_write_pos: AtomicUsize::new(0),
            output_read_pos: AtomicUsize::new(0),
        }
    }

    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Audio thread only. Returns true when buffer is full.
    #[inline]
    pub fn write_input(&self, channel: usize, sample: f32) -> bool {
        let write_idx = self.input_write_idx.load(Ordering::Acquire);
        let pos = self.input_write_pos.load(Ordering::Relaxed);

        let buffer = unsafe { &mut *self.input_buffers[write_idx].get() };
        let offset = pos * self.channels + channel;

        if offset < buffer.len() {
            buffer[offset] = sample;
        }

        if channel == self.channels - 1 {
            let new_pos = pos + 1;
            self.input_write_pos.store(new_pos, Ordering::Relaxed);

            if new_pos >= self.buffer_size {
                self.input_write_pos.store(0, Ordering::Relaxed);
                self.input_ready.store(true, Ordering::Release);
                self.input_write_idx.store(1 - write_idx, Ordering::Release);
                return true;
            }
        }

        false
    }

    /// Audio thread only. Returns 0.0 if no output ready.
    #[inline]
    pub fn read_output(&self, channel: usize) -> f32 {
        if !self.output_ready.load(Ordering::Acquire) {
            return 0.0;
        }

        let read_idx = self.output_read_idx.load(Ordering::Acquire);
        let pos = self.output_read_pos.load(Ordering::Relaxed);

        let buffer = unsafe { &*self.output_buffers[read_idx].get() };
        let offset = pos * self.channels + channel;

        let sample = if offset < buffer.len() {
            buffer[offset]
        } else {
            0.0
        };

        if channel == self.channels - 1 {
            let new_pos = pos + 1;
            self.output_read_pos.store(new_pos, Ordering::Relaxed);

            if new_pos >= self.buffer_size {
                self.output_read_pos.store(0, Ordering::Relaxed);
                self.output_ready.store(false, Ordering::Release);
                self.output_read_idx.store(1 - read_idx, Ordering::Release);
            }
        }

        sample
    }

    #[inline]
    pub fn has_output(&self) -> bool {
        self.output_ready.load(Ordering::Acquire)
    }

    #[inline]
    pub fn has_input(&self) -> bool {
        self.input_ready.load(Ordering::Acquire)
    }

    /// Inference thread only.
    pub fn take_input(&self) -> Option<&[f32]> {
        if !self.input_ready.load(Ordering::Acquire) {
            return None;
        }

        let write_idx = self.input_write_idx.load(Ordering::Acquire);
        let read_idx = 1 - write_idx;
        self.input_ready.store(false, Ordering::Release);

        Some(unsafe { &*self.input_buffers[read_idx].get() })
    }

    /// Inference thread only.
    pub fn write_output(&self, data: &[f32]) {
        let read_idx = self.output_read_idx.load(Ordering::Acquire);
        let write_idx = 1 - read_idx;

        let buffer = unsafe { &mut *self.output_buffers[write_idx].get() };
        let len = data.len().min(buffer.len());
        buffer[..len].copy_from_slice(&data[..len]);

        self.output_ready.store(true, Ordering::Release);
        self.output_read_idx.store(write_idx, Ordering::Release);
        self.output_read_pos.store(0, Ordering::Release);
    }
}

// SAFETY: SPSC protocol with Release/Acquire ordering ensures no data races.
unsafe impl Send for EffectAudioQueue {}
unsafe impl Sync for EffectAudioQueue {}

pub type SharedEffectAudioQueue = Arc<EffectAudioQueue>;

pub fn shared_effect_queue(channels: usize, buffer_size: usize) -> SharedEffectAudioQueue {
    Arc::new(EffectAudioQueue::new(channels, buffer_size))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_creation() {
        let queue = EffectAudioQueue::new(2, 512);
        assert_eq!(queue.channels(), 2);
        assert_eq!(queue.buffer_size(), 512);
        assert!(!queue.has_input());
        assert!(!queue.has_output());
    }

    #[test]
    fn test_write_read_cycle() {
        let queue = EffectAudioQueue::new(2, 4);

        // Write 4 stereo samples
        for i in 0..4 {
            let sample = i as f32 * 0.1;
            queue.write_input(0, sample);
            let ready = queue.write_input(1, sample);
            assert_eq!(ready, i == 3);
        }

        assert!(queue.has_input());

        let input = queue.take_input().unwrap();
        assert_eq!(input.len(), 8);

        let processed: Vec<f32> = input.iter().map(|x| x * 2.0).collect();
        queue.write_output(&processed);

        assert!(queue.has_output());

        for i in 0..4 {
            let expected = i as f32 * 0.1 * 2.0;
            assert!((queue.read_output(0) - expected).abs() < 0.001);
            assert!((queue.read_output(1) - expected).abs() < 0.001);
        }
    }
}
