//! Neural synthesizer with lock-free parameter passing.

use crate::gpu::{
    ControlParams, InferenceRequest, MidiState, NeuralModelId, NeuralParamQueue, VoiceId,
};
use std::sync::Arc;

/// Neural synthesizer.
pub struct NeuralSynth {
    pub(crate) track_id: VoiceId,
    pub(crate) model_id: NeuralModelId,
    pub(crate) param_queue: Arc<NeuralParamQueue>,
    pub(crate) current_params: ControlParams,
    pub(crate) sample_rate: f32,
    pub(crate) buffer_size: usize,
    pub(crate) phase: f32,
    pub(crate) midi_state: MidiState,
    pub(crate) midi_tx: crossbeam_channel::Sender<InferenceRequest>,
}

impl NeuralSynth {
    /// Create a new neural synth.
    pub fn new(
        track_id: VoiceId,
        model_id: NeuralModelId,
        param_queue: Arc<NeuralParamQueue>,
        sample_rate: f32,
        buffer_size: usize,
        midi_tx: crossbeam_channel::Sender<InferenceRequest>,
    ) -> Self {
        let current_params = ControlParams {
            f0: vec![440.0; buffer_size],
            amplitudes: vec![0.0; buffer_size],
        };

        Self {
            track_id,
            model_id,
            param_queue,
            current_params,
            sample_rate,
            buffer_size,
            phase: 0.0,
            midi_state: MidiState::default(),
            midi_tx,
        }
    }

    /// Update control parameters from the queue (non-blocking).
    pub fn update_params_from_queue(&mut self) {
        if let Some(new_params) = self.param_queue.try_pop() {
            self.current_params = new_params;
            tracing::trace!(
                "Track {}: Updated params (f0={}, amp={})",
                self.track_id,
                self.current_params.f0.len(),
                self.current_params.amplitudes.len()
            );
        }
    }
}

unsafe impl Sync for NeuralSynth {}

impl tutti_core::MidiAudioUnit for NeuralSynth {
    /// Queue MIDI events for neural processing (non-blocking).
    fn queue_midi(&mut self, events: &[tutti_core::MidiEvent]) {
        for event in events {
            let triggers_inference = self.midi_state.apply(event);

            if triggers_inference {
                let features = self.midi_state.to_features();
                let request = InferenceRequest::from_slice(
                    self.track_id,
                    self.model_id,
                    &features,
                    self.buffer_size,
                );

                // Drop MIDI event if queue is full (non-blocking)
                let _ = self.midi_tx.try_send(request);
            }
        }
    }

    fn has_midi_output(&self) -> bool {
        false
    }

    fn clear_midi(&mut self) {
        self.midi_state = MidiState::default();
    }
}
