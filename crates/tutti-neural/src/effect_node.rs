//! Neural effect AudioUnit — audio → tensor → engine → audio.
//!
//! Stereo input, stereo output. Latency = buffer_size samples (reported for PDC).

use crate::engine::{submit_request, ResponseChannel, TensorRequest};
use crate::gpu::effect_queue::SharedEffectAudioQueue;
use crate::gpu::NeuralModelId;
use crossbeam_channel::Sender;
use std::sync::atomic::{AtomicU64, Ordering};
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

/// Counter for generating unique instance IDs.
static INSTANCE_COUNTER: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// Struct
// ============================================================================

/// Neural effect AudioUnit.
///
/// Double-buffered lock-free pipeline:
/// 1. Audio thread writes input samples into the queue
/// 2. When buffer is full, submits a TensorRequest to the engine
/// 3. Engine runs inference and writes processed audio back to the queue
/// 4. Audio thread reads processed samples from the queue
///
/// Latency = buffer_size samples, reported via `latency()` for PDC.
pub struct NeuralEffectNode {
    instance_id: u64,
    model_id: NeuralModelId,
    buffer_size: usize,
    sample_rate: f32,
    channels: usize,
    audio_queue: SharedEffectAudioQueue,
    request_tx: Sender<TensorRequest>,
}

// ============================================================================
// Construction
// ============================================================================

impl NeuralEffectNode {
    pub fn new(
        model_id: NeuralModelId,
        buffer_size: usize,
        audio_queue: SharedEffectAudioQueue,
        request_tx: Sender<TensorRequest>,
    ) -> Self {
        Self {
            instance_id: INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed),
            model_id,
            buffer_size,
            sample_rate: 44100.0,
            channels: 2,
            audio_queue,
            request_tx,
        }
    }

    pub fn with_sample_rate(mut self, sample_rate: f32) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// When the input buffer is full, take the data and submit to the engine.
    #[inline]
    fn maybe_submit(&self) {
        if let Some(input_data) = self.audio_queue.take_input() {
            let _ = submit_request(
                &self.request_tx,
                TensorRequest {
                    model_id: self.model_id,
                    input: input_data.to_vec(),
                    input_shape: [1, input_data.len()],
                    response: ResponseChannel::Audio(self.audio_queue.clone()),
                },
            );
        }
    }
}

// ============================================================================
// AudioUnit
// ============================================================================

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

// ============================================================================
// Clone
// ============================================================================

impl Clone for NeuralEffectNode {
    fn clone(&self) -> Self {
        Self {
            instance_id: INSTANCE_COUNTER.fetch_add(1, Ordering::Relaxed),
            model_id: self.model_id,
            buffer_size: self.buffer_size,
            sample_rate: self.sample_rate,
            channels: self.channels,
            audio_queue: self.audio_queue.clone(),
            request_tx: self.request_tx.clone(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

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
        let node = NeuralEffectNode::new(model_id, 4, queue, tx);

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
