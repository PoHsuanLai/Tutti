//! Lock-free audio buffers for neural effect processing.
//!
//! Provides SPSC queues for transferring audio between the audio thread
//! and the inference thread. Effects need bidirectional audio transfer:
//! - Input: audio thread → inference thread
//! - Output: inference thread → audio thread
//!
//! RT-safe: No locks, only atomic operations.

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

/// Audio buffer for neural effect processing.
///
/// Double-buffered design for lock-free audio transfer:
/// - Audio thread writes to input buffer, reads from output buffer
/// - Inference thread reads from input buffer, writes to output buffer
/// - Atomic flags coordinate buffer swaps
///
/// # Safety
/// This is a SPSC (single-producer, single-consumer) queue per direction:
/// - Input: audio thread produces, inference thread consumes
/// - Output: inference thread produces, audio thread consumes
///
/// The atomics ensure proper synchronization. UnsafeCell is used because
/// we guarantee exclusive access through the protocol.
pub struct EffectAudioQueue {
    /// Input buffers (audio thread writes, inference reads)
    /// UnsafeCell because audio thread writes, inference thread reads (no overlap)
    input_buffers: [UnsafeCell<Vec<f32>>; 2],

    /// Output buffers (inference writes, audio thread reads)
    /// UnsafeCell because inference thread writes, audio thread reads (no overlap)
    output_buffers: [UnsafeCell<Vec<f32>>; 2],

    /// Which input buffer is being written by audio thread (0 or 1)
    input_write_idx: AtomicUsize,

    /// Which output buffer is being read by audio thread (0 or 1)
    output_read_idx: AtomicUsize,

    /// Input buffer ready for inference (set by audio thread, cleared by inference)
    input_ready: AtomicBool,

    /// Output buffer ready for audio thread (set by inference, cleared by audio thread)
    output_ready: AtomicBool,

    /// Number of channels
    channels: usize,

    /// Buffer size in samples per channel
    buffer_size: usize,

    /// Write position within current input buffer
    input_write_pos: AtomicUsize,

    /// Read position within current output buffer
    output_read_pos: AtomicUsize,
}

impl EffectAudioQueue {
    /// Create a new effect audio queue.
    ///
    /// # Arguments
    /// * `channels` - Number of audio channels (typically 2 for stereo)
    /// * `buffer_size` - Samples per channel per buffer (e.g., 512)
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

    /// Get buffer size in samples per channel.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Get number of channels.
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Write a sample to the input buffer (audio thread only).
    ///
    /// Returns true if a complete buffer is ready for inference.
    ///
    /// # Safety
    /// Must only be called from the audio thread.
    #[inline]
    pub fn write_input(&self, channel: usize, sample: f32) -> bool {
        let write_idx = self.input_write_idx.load(Ordering::Acquire);
        let pos = self.input_write_pos.load(Ordering::Relaxed);

        // Safety: audio thread has exclusive write access to the current write buffer
        let buffer = unsafe { &mut *self.input_buffers[write_idx].get() };
        let offset = pos * self.channels + channel;

        if offset < buffer.len() {
            buffer[offset] = sample;
        }

        // Only advance position after writing last channel
        if channel == self.channels - 1 {
            let new_pos = pos + 1;
            self.input_write_pos.store(new_pos, Ordering::Relaxed);

            // Buffer full - signal ready and swap
            if new_pos >= self.buffer_size {
                self.input_write_pos.store(0, Ordering::Relaxed);
                self.input_ready.store(true, Ordering::Release);

                // Swap to other buffer
                let next_idx = 1 - write_idx;
                self.input_write_idx.store(next_idx, Ordering::Release);

                return true;
            }
        }

        false
    }

    /// Read a sample from the output buffer (audio thread only).
    ///
    /// Returns the processed sample, or 0.0 if no output is ready yet.
    ///
    /// # Safety
    /// Must only be called from the audio thread.
    #[inline]
    pub fn read_output(&self, channel: usize) -> f32 {
        // Check if output is ready
        if !self.output_ready.load(Ordering::Acquire) {
            return 0.0;
        }

        let read_idx = self.output_read_idx.load(Ordering::Acquire);
        let pos = self.output_read_pos.load(Ordering::Relaxed);

        // Safety: audio thread has exclusive read access to the current read buffer
        let buffer = unsafe { &*self.output_buffers[read_idx].get() };
        let offset = pos * self.channels + channel;

        let sample = if offset < buffer.len() {
            buffer[offset]
        } else {
            0.0
        };

        // Only advance position after reading last channel
        if channel == self.channels - 1 {
            let new_pos = pos + 1;
            self.output_read_pos.store(new_pos, Ordering::Relaxed);

            // Buffer consumed - signal not ready and swap
            if new_pos >= self.buffer_size {
                self.output_read_pos.store(0, Ordering::Relaxed);
                self.output_ready.store(false, Ordering::Release);

                // Swap to other buffer
                let next_idx = 1 - read_idx;
                self.output_read_idx.store(next_idx, Ordering::Release);
            }
        }

        sample
    }

    /// Check if output is ready to read (audio thread).
    #[inline]
    pub fn has_output(&self) -> bool {
        self.output_ready.load(Ordering::Acquire)
    }

    /// Check if input is ready for inference.
    #[inline]
    pub fn has_input(&self) -> bool {
        self.input_ready.load(Ordering::Acquire)
    }

    /// Take the input buffer for inference (inference thread only).
    ///
    /// Returns the buffer data if ready, None otherwise.
    /// Clears the ready flag.
    ///
    /// # Safety
    /// Must only be called from the inference thread.
    pub fn take_input(&self) -> Option<&[f32]> {
        if !self.input_ready.load(Ordering::Acquire) {
            return None;
        }

        // Read from the buffer that was just completed (opposite of write idx)
        let write_idx = self.input_write_idx.load(Ordering::Acquire);
        let read_idx = 1 - write_idx;

        self.input_ready.store(false, Ordering::Release);

        // Safety: inference thread has exclusive read access to the completed buffer
        Some(unsafe { &*self.input_buffers[read_idx].get() })
    }

    /// Write processed output (inference thread only).
    ///
    /// Copies the processed audio to the output buffer and signals ready.
    ///
    /// # Safety
    /// Must only be called from the inference thread.
    pub fn write_output(&self, data: &[f32]) {
        // Write to the buffer that's not being read
        let read_idx = self.output_read_idx.load(Ordering::Acquire);
        let write_idx = 1 - read_idx;

        // Safety: inference thread has exclusive write access to the non-read buffer
        let buffer = unsafe { &mut *self.output_buffers[write_idx].get() };
        let len = data.len().min(buffer.len());
        buffer[..len].copy_from_slice(&data[..len]);

        // Signal output ready
        self.output_ready.store(true, Ordering::Release);

        // Swap buffers for next read
        self.output_read_idx.store(write_idx, Ordering::Release);
        self.output_read_pos.store(0, Ordering::Release);
    }
}

// Safety: EffectAudioQueue uses UnsafeCell with atomics for synchronization.
// The SPSC protocol guarantees no data races:
// - Input: audio thread writes to buffer[write_idx], inference reads buffer[1-write_idx]
// - Output: inference writes to buffer[1-read_idx], audio reads buffer[read_idx]
// Atomics with Release/Acquire ordering ensure proper synchronization.
unsafe impl Send for EffectAudioQueue {}
unsafe impl Sync for EffectAudioQueue {}

/// Shared effect audio queue wrapped in Arc for cross-thread access.
/// No locks - all synchronization is via atomics inside EffectAudioQueue.
pub type SharedEffectAudioQueue = Arc<EffectAudioQueue>;

/// Create a new shared effect audio queue.
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
        let queue = EffectAudioQueue::new(2, 4); // Small buffer for testing

        // Write 4 stereo samples (fills one buffer)
        for i in 0..4 {
            let sample = i as f32 * 0.1;
            queue.write_input(0, sample); // Left
            let ready = queue.write_input(1, sample); // Right

            // Last sample should signal ready
            if i == 3 {
                assert!(ready);
            } else {
                assert!(!ready);
            }
        }

        // Input should be ready now
        assert!(queue.has_input());

        // Take input for "processing"
        let input = queue.take_input().unwrap();
        assert_eq!(input.len(), 8); // 4 samples * 2 channels

        // Simulate processed output
        let processed: Vec<f32> = input.iter().map(|x| x * 2.0).collect();
        queue.write_output(&processed);

        // Output should be ready
        assert!(queue.has_output());

        // Read back
        for i in 0..4 {
            let expected = i as f32 * 0.1 * 2.0;
            let left = queue.read_output(0);
            let right = queue.read_output(1);
            assert!((left - expected).abs() < 0.001);
            assert!((right - expected).abs() < 0.001);
        }
    }
}
