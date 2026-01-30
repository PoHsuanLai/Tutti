//! Audio processing implementation for neural synth
//!
//! Implements the AudioUnit trait for real-time neural audio generation.
//! Uses Neural synthesis: harmonic oscillators + filtered noise.

use super::neural_synth::NeuralSynth;
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

/// AudioUnit implementation for NeuralSynth
///
/// This allows NeuralSynth to be used as a node in FunDSP Net.
impl AudioUnit for NeuralSynth {
    fn inputs(&self) -> usize {
        0 // Synth is a source, no audio inputs
    }

    fn outputs(&self) -> usize {
        2 // Stereo output
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        // For single-sample processing, use current params
        self.update_params_from_queue();

        let params = &self.current_params;
        if params.f0.is_empty() || params.amplitudes.is_empty() {
            // No params yet - output silence
            for sample in output.iter_mut() {
                *sample = 0.0;
            }
            return;
        }

        let f0 = params.f0[0];
        let amplitude = params.amplitudes[0];
        let two_pi = 2.0 * std::f32::consts::PI;

        // Generate sample
        let sample = amplitude * self.phase.sin();

        // Advance phase
        let phase_increment = (f0 / self.sample_rate) * two_pi;
        self.phase += phase_increment;
        if self.phase >= two_pi {
            self.phase -= two_pi;
        }

        // Write to output (stereo)
        if output.len() >= 2 {
            output[0] = sample;
            output[1] = sample;
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        // Update control params from queue (lock-free!)
        self.update_params_from_queue();

        let params = &self.current_params;

        // Ensure we have enough params for this buffer
        let num_samples = size.min(params.f0.len()).min(params.amplitudes.len());

        if num_samples == 0 {
            // No params yet - output silence
            for i in 0..size {
                output.set_f32(0, i, 0.0);
                output.set_f32(1, i, 0.0);
            }
            return;
        }

        // Phase accumulator - continuous across buffers
        let mut phase = self.phase;
        let two_pi = 2.0 * std::f32::consts::PI;

        // Simplified neural synthesis (harmonic oscillator)
        for i in 0..num_samples {
            let f0 = params.f0[i];
            let amplitude = params.amplitudes[i];

            // Generate sine wave sample at current phase
            let sample = amplitude * phase.sin();

            // Advance phase based on frequency (f0)
            let phase_increment = (f0 / self.sample_rate) * two_pi;
            phase += phase_increment;

            // Wrap phase to prevent accumulation errors
            if phase >= two_pi {
                phase -= two_pi;
            }

            // Write to stereo output
            output.set_f32(0, i, sample);
            output.set_f32(1, i, sample);
        }

        // Store phase for next buffer (continuous synthesis!)
        self.phase = phase;

        // Fill remaining samples with silence if params were shorter
        for i in num_samples..size {
            output.set_f32(0, i, 0.0);
            output.set_f32(1, i, 0.0);
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn reset(&mut self) {
        // Reset phase to prevent clicks on restart
        self.phase = 0.0;
        tracing::debug!("Neural synth reset (track {})", self.track_id);
    }

    fn get_id(&self) -> u64 {
        // Use track_id as part of unique identifier
        self.track_id
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        // Neural synth is a source - output stereo
        SignalFrame::new(2)
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.current_params.f0.len() * std::mem::size_of::<f32>()
            + self.current_params.amplitudes.len() * std::mem::size_of::<f32>()
    }
}

/// Clone implementation for NeuralSynth (required for DynClone)
impl Clone for NeuralSynth {
    fn clone(&self) -> Self {
        NeuralSynth {
            track_id: self.track_id,
            model_id: self.model_id,
            param_queue: std::sync::Arc::clone(&self.param_queue),
            current_params: self.current_params.clone(),
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
            phase: self.phase,
            midi_state: self.midi_state.clone(),
            midi_tx: self.midi_tx.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{ControlParams, NeuralModelId, NeuralParamQueue};
    use std::sync::Arc;

    #[test]
    fn test_neural_synth_outputs() {
        let param_queue = Arc::new(NeuralParamQueue::new(16));
        let (midi_tx, _midi_rx) = crossbeam_channel::unbounded();
        let synth = NeuralSynth::new(0, NeuralModelId::new(), param_queue, 44100.0, 512, midi_tx);

        // Synth should have stereo output
        assert_eq!(synth.outputs(), 2);
        assert_eq!(synth.inputs(), 0);
    }

    #[test]
    fn test_lock_free_param_update() {
        // Use raw queue (without taking sender) so we can use try_push()
        let param_queue = Arc::new(NeuralParamQueue::new(16));
        let (midi_tx, _midi_rx) = crossbeam_channel::unbounded();
        let mut synth = NeuralSynth::new(
            0,
            NeuralModelId::new(),
            param_queue.clone(),
            44100.0,
            512,
            midi_tx,
        );

        // Push new params via try_push (sender not taken in this test)
        let new_params = ControlParams {
            f0: vec![220.0; 512],
            amplitudes: vec![0.5; 512],
        };
        param_queue
            .try_push(new_params)
            .expect("push should succeed");

        // Update should be non-blocking
        synth.update_params_from_queue();

        // Check params were updated
        assert_eq!(synth.current_params.f0[0], 220.0);
        assert_eq!(synth.current_params.amplitudes[0], 0.5);
    }
}
