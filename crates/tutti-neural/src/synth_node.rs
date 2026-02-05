//! Neural synth AudioUnit — MIDI → tensor → ControlParams → audio.
//!
//! Zero inputs, stereo output. No processing latency (params arrive async).
//!
//! ## RT Safety
//!
//! Uses pre-allocated Arc buffers to avoid heap allocation in the audio callback.
//! The feature vector is small (12 floats), so we use a simple buffer pool.

use crate::engine::{submit_request, ResponseChannel, TensorRequest};
use crate::gpu::{ControlParams, MidiState, NeuralModelId, MIDI_FEATURE_COUNT};
use crossbeam_channel::{Receiver, Sender};
use std::sync::Arc;
use tutti_core::midi::{MidiEvent, MidiRegistry};
use tutti_core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

/// Pool of pre-allocated Arc<[f32]> buffers for RT-safe tensor submission.
///
/// For synth nodes, the feature vector is small (MIDI_FEATURE_COUNT = 12 floats).
/// We use a small pool since MIDI events are sparse compared to audio buffers.
const SYNTH_POOL_SIZE: usize = 4;

struct SynthBufferPool {
    buffers: [Option<Arc<[f32]>>; SYNTH_POOL_SIZE],
    next_slot: usize,
}

impl SynthBufferPool {
    fn new() -> Self {
        // Pre-allocate buffers for MIDI feature vectors
        let buffers = std::array::from_fn(|_| Some(Arc::from(vec![0.0f32; MIDI_FEATURE_COUNT])));
        Self {
            buffers,
            next_slot: 0,
        }
    }

    /// Get a buffer and fill it with the given data. RT-safe (no allocation).
    #[inline]
    fn get_and_fill(&mut self, data: &[f32; MIDI_FEATURE_COUNT]) -> Option<Arc<[f32]>> {
        for _ in 0..SYNTH_POOL_SIZE {
            let slot = self.next_slot;
            self.next_slot = (self.next_slot + 1) % SYNTH_POOL_SIZE;

            if let Some(arc) = self.buffers[slot].take() {
                if Arc::strong_count(&arc) == 1 {
                    let mut arc = arc;
                    let buf = Arc::make_mut(&mut arc);
                    buf.copy_from_slice(data);

                    let result = Arc::clone(&arc);
                    self.buffers[slot] = Some(arc);
                    return Some(result);
                } else {
                    self.buffers[slot] = Some(arc);
                }
            }
        }
        None
    }
}

/// Neural synthesizer AudioUnit.
///
/// Receives ControlParams from the inference engine via crossbeam_channel.
/// Each MIDI event that triggers inference submits a TensorRequest with
/// a cloned Sender so the engine can push results back.
///
/// MIDI events are polled from the [`MidiRegistry`] during audio processing
/// (pull-based, same pattern as `SoundFontUnit`).
///
/// ## RT Safety
///
/// Uses `SynthBufferPool` to avoid heap allocation when submitting requests.
pub struct NeuralSynthNode {
    model_id: NeuralModelId,
    param_tx: Sender<ControlParams>,
    param_rx: Receiver<ControlParams>,
    current_params: ControlParams,
    sample_rate: f32,
    buffer_size: usize,
    phase: f32,
    midi_state: MidiState,
    request_tx: Sender<TensorRequest>,
    midi_registry: Option<MidiRegistry>,
    midi_buffer: Vec<MidiEvent>,
    /// Pre-allocated buffer pool for RT-safe tensor submission
    buffer_pool: SynthBufferPool,
}

impl NeuralSynthNode {
    pub fn new(
        model_id: NeuralModelId,
        sample_rate: f32,
        buffer_size: usize,
        request_tx: Sender<TensorRequest>,
    ) -> Self {
        let (param_tx, param_rx) = crossbeam_channel::bounded::<ControlParams>(16);

        Self {
            model_id,
            param_tx,
            param_rx,
            current_params: ControlParams {
                f0: vec![440.0; buffer_size],
                amplitudes: vec![0.0; buffer_size],
            },
            sample_rate,
            buffer_size,
            phase: 0.0,
            midi_state: MidiState::default(),
            request_tx,
            midi_registry: None,
            midi_buffer: vec![MidiEvent::note_on(0, 0, 0, 0); 256],
            buffer_pool: SynthBufferPool::new(),
        }
    }

    /// Set the MIDI registry for pull-based MIDI event delivery.
    pub fn with_midi_registry(mut self, registry: MidiRegistry) -> Self {
        self.midi_registry = Some(registry);
        self
    }

    /// Poll MIDI events from the registry and process them.
    ///
    /// Called at the start of `tick()` and `process()` to receive MIDI events
    /// that were queued via `engine.queue_midi()`.
    fn poll_midi_events(&mut self) {
        let registry = match &self.midi_registry {
            Some(r) => r,
            None => return,
        };

        let unit_id = self.model_id.as_u64();
        let count = registry.poll_into(unit_id, &mut self.midi_buffer);

        for i in 0..count {
            let event = &self.midi_buffer[i];
            if self.midi_state.apply(event) {
                self.submit_inference_request();
            }
        }
    }

    /// Submit an inference request with current MIDI state.
    ///
    /// RT-safe: Uses pre-allocated buffer pool, no heap allocation.
    fn submit_inference_request(&mut self) {
        let features = self.midi_state.to_features();
        // Get a pre-allocated buffer from the pool (RT-safe)
        if let Some(arc_buffer) = self.buffer_pool.get_and_fill(&features) {
            let _ = submit_request(
                &self.request_tx,
                TensorRequest {
                    model_id: self.model_id,
                    input: arc_buffer,
                    input_shape: [1, MIDI_FEATURE_COUNT],
                    response: ResponseChannel::Params {
                        sender: self.param_tx.clone(),
                        buffer_size: self.buffer_size,
                    },
                },
            );
        }
        // If pool exhausted, drop this request (RT-safe behavior)
    }

    /// Drain the param channel, keeping only the latest.
    fn update_params(&mut self) {
        while let Ok(params) = self.param_rx.try_recv() {
            self.current_params = params;
        }
    }
}

impl AudioUnit for NeuralSynthNode {
    fn inputs(&self) -> usize {
        0
    }

    fn outputs(&self) -> usize {
        2
    }

    fn tick(&mut self, _input: &[f32], output: &mut [f32]) {
        self.poll_midi_events();
        self.update_params();

        let params = &self.current_params;
        if params.f0.is_empty() || params.amplitudes.is_empty() {
            for s in output.iter_mut() {
                *s = 0.0;
            }
            return;
        }

        let two_pi = 2.0 * std::f32::consts::PI;
        let sample = params.amplitudes[0] * self.phase.sin();

        self.phase += (params.f0[0] / self.sample_rate) * two_pi;
        if self.phase >= two_pi {
            self.phase -= two_pi;
        }

        if output.len() >= 2 {
            output[0] = sample;
            output[1] = sample;
        }
    }

    fn process(&mut self, size: usize, _input: &BufferRef, output: &mut BufferMut) {
        self.poll_midi_events();
        self.update_params();

        let params = &self.current_params;
        let n = size.min(params.f0.len()).min(params.amplitudes.len());

        if n == 0 {
            for i in 0..size {
                output.set_f32(0, i, 0.0);
                output.set_f32(1, i, 0.0);
            }
            return;
        }

        let two_pi = 2.0 * std::f32::consts::PI;
        let mut phase = self.phase;

        for i in 0..n {
            let sample = params.amplitudes[i] * phase.sin();
            phase += (params.f0[i] / self.sample_rate) * two_pi;
            if phase >= two_pi {
                phase -= two_pi;
            }
            output.set_f32(0, i, sample);
            output.set_f32(1, i, sample);
        }

        self.phase = phase;

        for i in n..size {
            output.set_f32(0, i, 0.0);
            output.set_f32(1, i, 0.0);
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn get_id(&self) -> u64 {
        self.model_id.as_u64()
    }

    fn route(&mut self, _input: &SignalFrame, _frequency: f64) -> SignalFrame {
        SignalFrame::new(2)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn footprint(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.current_params.f0.len() * std::mem::size_of::<f32>()
            + self.current_params.amplitudes.len() * std::mem::size_of::<f32>()
    }
}

impl Clone for NeuralSynthNode {
    fn clone(&self) -> Self {
        Self {
            model_id: self.model_id,
            param_tx: self.param_tx.clone(),
            param_rx: self.param_rx.clone(),
            current_params: self.current_params.clone(),
            sample_rate: self.sample_rate,
            buffer_size: self.buffer_size,
            phase: self.phase,
            midi_state: self.midi_state.clone(),
            request_tx: self.request_tx.clone(),
            midi_registry: self.midi_registry.clone(),
            midi_buffer: vec![MidiEvent::note_on(0, 0, 0, 0); 256],
            // Each clone gets its own buffer pool (pre-allocated at clone time, not RT)
            buffer_pool: SynthBufferPool::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synth_node_io() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let node = NeuralSynthNode::new(NeuralModelId::new(), 44100.0, 512, tx);
        assert_eq!(node.inputs(), 0);
        assert_eq!(node.outputs(), 2);
    }

    #[test]
    fn test_synth_node_param_update() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let mut node = NeuralSynthNode::new(NeuralModelId::new(), 44100.0, 512, tx);

        node.param_tx
            .send(ControlParams {
                f0: vec![220.0; 512],
                amplitudes: vec![0.5; 512],
            })
            .unwrap();

        node.update_params();
        assert_eq!(node.current_params.f0[0], 220.0);
        assert_eq!(node.current_params.amplitudes[0], 0.5);
    }
}
