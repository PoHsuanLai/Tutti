//! Neural effect AudioUnit — audio → tensor → engine → audio.
//!
//! Stereo input, stereo output. Latency = buffer_size samples (reported for PDC).
//!
//! ## RT Safety
//!
//! Uses pre-allocated Arc buffers to avoid heap allocation in the audio callback.
//! The `ArcBufferPool` maintains a small pool of reusable Arc<[f32]> buffers.

use crate::engine::{submit_request, ResponseChannel, TensorRequest};
use crate::gpu::effect_queue::SharedEffectAudioQueue;
use crate::gpu::NeuralModelId;
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

/// Pool of pre-allocated Arc<[f32]> buffers for RT-safe tensor submission.
///
/// The pool uses a simple ring buffer. When a buffer is needed:
/// 1. Check if the current slot's Arc has refcount == 1 (only pool owns it)
/// 2. If yes, reuse it via Arc::make_mut (no allocation)
/// 3. If no (inference thread still using it), move to next slot
///
/// Pool size of 4 handles typical inference latency (2-3 buffers in flight).
const POOL_SIZE: usize = 4;

struct ArcBufferPool {
    buffers: [Option<Arc<[f32]>>; POOL_SIZE],
    next_slot: usize,
}

impl ArcBufferPool {
    fn new(buffer_size: usize) -> Self {
        // Pre-allocate all buffers at construction time
        let buffers = std::array::from_fn(|_| Some(Arc::from(vec![0.0f32; buffer_size])));
        Self {
            buffers,
            next_slot: 0,
        }
    }

    /// Get a buffer and fill it with the given data. RT-safe (no allocation).
    ///
    /// Returns None if all buffers are in use (inference thread is backed up).
    #[inline]
    fn get_and_fill(&mut self, data: &[f32]) -> Option<Arc<[f32]>> {
        // Try each slot in the pool
        for _ in 0..POOL_SIZE {
            let slot = self.next_slot;
            self.next_slot = (self.next_slot + 1) % POOL_SIZE;

            if let Some(arc) = self.buffers[slot].take() {
                // Check if we're the only owner (refcount == 1)
                if Arc::strong_count(&arc) == 1 {
                    // We can reuse this buffer - Arc::make_mut won't allocate
                    let mut arc = arc;
                    let buf = Arc::make_mut(&mut arc);
                    let copy_len = data.len().min(buf.len());
                    buf[..copy_len].copy_from_slice(&data[..copy_len]);

                    // Put it back and return a clone
                    let result = Arc::clone(&arc);
                    self.buffers[slot] = Some(arc);
                    return Some(result);
                } else {
                    // Inference thread still has a reference, put it back
                    self.buffers[slot] = Some(arc);
                }
            }
        }

        // All buffers in use - drop this request (RT-safe behavior)
        None
    }
}

/// Counter for generating unique instance IDs.
static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Neural effect AudioUnit.
///
/// Double-buffered lock-free pipeline:
/// 1. Audio thread writes input samples into the queue
/// 2. When buffer is full, submits a TensorRequest to the engine
/// 3. Engine runs inference and writes processed audio back to the queue
/// 4. Audio thread reads processed samples from the queue
///
/// Latency = buffer_size samples, reported via `latency()` for PDC.
///
/// ## RT Safety
///
/// Uses `ArcBufferPool` to avoid heap allocation when submitting requests.
/// The pool pre-allocates Arc buffers at construction time and reuses them.
pub struct NeuralEffectNode {
    instance_id: u64,
    model_id: NeuralModelId,
    buffer_size: usize,
    sample_rate: f32,
    channels: usize,
    audio_queue: SharedEffectAudioQueue,
    request_tx: Sender<TensorRequest>,
    /// Pre-allocated buffer pool for RT-safe tensor submission
    buffer_pool: ArcBufferPool,
}

impl NeuralEffectNode {
    pub fn new(
        model_id: NeuralModelId,
        buffer_size: usize,
        audio_queue: SharedEffectAudioQueue,
        request_tx: Sender<TensorRequest>,
    ) -> Self {
        let channels = 2;
        // Pre-allocate buffer pool with total_samples = channels * buffer_size
        let total_samples = channels * buffer_size;
        Self {
            instance_id: INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed),
            model_id,
            buffer_size,
            sample_rate: 44100.0,
            channels,
            audio_queue,
            request_tx,
            buffer_pool: ArcBufferPool::new(total_samples),
        }
    }

    pub fn with_sample_rate(mut self, sample_rate: f32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// When the input buffer is full, take the data and submit to the engine.
    ///
    /// RT-safe: Uses pre-allocated buffer pool, no heap allocation.
    #[inline]
    fn maybe_submit(&mut self) {
        if let Some(input_data) = self.audio_queue.take_input() {
            // Get a pre-allocated buffer from the pool (RT-safe)
            if let Some(arc_buffer) = self.buffer_pool.get_and_fill(input_data) {
                let input_len = input_data.len();
                let _ = submit_request(
                    &self.request_tx,
                    TensorRequest {
                        model_id: self.model_id,
                        input: arc_buffer,
                        input_shape: [1, input_len],
                        response: ResponseChannel::Audio(self.audio_queue.clone()),
                    },
                );
            }
            // If pool exhausted, drop this request (RT-safe behavior)
        }
    }
}

impl AudioUnit for NeuralEffectNode {
    fn inputs(&self) -> usize {
        self.channels
    }

    fn outputs(&self) -> usize {
        self.channels
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        for (ch, &sample) in input.iter().enumerate().take(self.channels) {
            self.audio_queue.write_input(ch, sample);
        }

        // Check if a full buffer is ready to submit
        self.maybe_submit();

        if self.audio_queue.has_output() {
            for ch in 0..self.channels.min(output.len()) {
                output[ch] = self.audio_queue.read_output(ch);
            }
        } else {
            for s in output.iter_mut() {
                *s = 0.0;
            }
        }
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        for i in 0..size {
            for ch in 0..self.channels {
                self.audio_queue.write_input(ch, input.at_f32(ch, i));
            }

            // Check after each sample-frame in case buffer fills mid-block
            self.maybe_submit();

            if self.audio_queue.has_output() {
                for ch in 0..self.channels {
                    output.set_f32(ch, i, self.audio_queue.read_output(ch));
                }
            } else {
                for ch in 0..self.channels {
                    output.set_f32(ch, i, 0.0);
                }
            }
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn reset(&mut self) {}

    fn get_id(&self) -> u64 {
        self.instance_id
    }

    fn latency(&mut self) -> Option<f64> {
        Some(self.buffer_size as f64)
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        input.clone()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Clone for NeuralEffectNode {
    fn clone(&self) -> Self {
        let total_samples = self.channels * self.buffer_size;
        Self {
            instance_id: INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed),
            model_id: self.model_id,
            buffer_size: self.buffer_size,
            sample_rate: self.sample_rate,
            channels: self.channels,
            audio_queue: self.audio_queue.clone(),
            request_tx: self.request_tx.clone(),
            // Each clone gets its own buffer pool (pre-allocated at clone time, not RT)
            buffer_pool: ArcBufferPool::new(total_samples),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::shared_effect_queue;

    #[test]
    fn test_effect_node_io() {
        let queue = shared_effect_queue(2, 512);
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut node = NeuralEffectNode::new(NeuralModelId::new(), 512, queue, tx);
        assert_eq!(node.inputs(), 2);
        assert_eq!(node.outputs(), 2);
        assert_eq!(node.latency(), Some(512.0));
    }

    #[test]
    fn test_effect_node_unique_ids() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let q1 = shared_effect_queue(2, 512);
        let q2 = shared_effect_queue(2, 512);
        let n1 = NeuralEffectNode::new(NeuralModelId::new(), 512, q1, tx.clone());
        let n2 = NeuralEffectNode::new(NeuralModelId::new(), 512, q2, tx);
        assert_ne!(n1.get_id(), n2.get_id());
    }

    #[test]
    fn test_effect_node_submits_request() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let queue = shared_effect_queue(2, 4); // 4 samples per channel
        let model_id = NeuralModelId::new();
        let mut node = NeuralEffectNode::new(model_id, 4, queue, tx);

        // Write 4 stereo sample-frames to fill the buffer
        for _ in 0..4 {
            node.audio_queue.write_input(0, 0.5);
            node.audio_queue.write_input(1, 0.5);
        }

        // Buffer should be full → take_input + submit
        node.maybe_submit();

        // Should have received exactly one TensorRequest
        let req = rx.try_recv().unwrap();
        assert_eq!(req.model_id, model_id);
        assert_eq!(req.input.len(), 8); // 4 samples * 2 channels
    }
}
