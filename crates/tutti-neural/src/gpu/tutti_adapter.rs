//! Tutti audio graph adapters
//!
//! Provides AudioUnit implementations for neural audio processing nodes.

use super::engine::NeuralModelId;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

// ============================================================================
// NeuralEffectNode - Effect processor node
// ============================================================================

/// Neural effect node that implements AudioUnit
///
/// Processes audio through neural inference. Audio is collected on the audio thread,
/// sent to the inference thread via channels, processed, and returned.
pub(crate) struct NeuralEffectNode {
    /// Model ID being used
    model_id: NeuralModelId,

    /// Buffer size for processing
    buffer_size: usize,

    /// Sample rate
    sample_rate: f32,

    /// Input channels
    input_channels: usize,

    /// Output channels
    output_channels: usize,
}

impl NeuralEffectNode {
    /// Create a new neural effect node
    pub fn new(model_id: NeuralModelId, buffer_size: usize) -> Self {
        Self {
            model_id,
            buffer_size,
            sample_rate: 44100.0,
            input_channels: 2,
            output_channels: 2,
        }
    }

    /// Set sample rate (builder pattern)
    pub fn with_sample_rate(mut self, sample_rate: f32) -> Self {
        self.sample_rate = sample_rate;
        self
    }
}

impl AudioUnit for NeuralEffectNode {
    fn inputs(&self) -> usize {
        self.input_channels
    }

    fn outputs(&self) -> usize {
        self.output_channels
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        // Simple passthrough for now (inference integration TODO)
        let len = input.len().min(output.len());
        output[..len].copy_from_slice(&input[..len]);
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        // Simple passthrough for now (inference integration TODO)
        for ch in 0..self.output_channels.min(self.input_channels) {
            for i in 0..size {
                let sample = input.at_f32(ch, i);
                output.set_f32(ch, i, sample);
            }
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn reset(&mut self) {
        // Reset state if needed
    }

    fn get_id(&self) -> u64 {
        self.model_id.as_u64()
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Pass through input channels
        input.clone()
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl Clone for NeuralEffectNode {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id,
            buffer_size: self.buffer_size,
            sample_rate: self.sample_rate,
            input_channels: self.input_channels,
            output_channels: self.output_channels,
        }
    }
}
