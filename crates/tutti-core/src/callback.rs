//! Real-time audio callback for FunDSP Net processing.

use crate::metering::MeteringManager;
use crate::transport::TransportManager;
use fundsp::audiounit::AudioUnit;
use fundsp::realnet::NetBackend;
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// State for the real-time audio callback.
/// Uses `UnsafeCell` for interior mutability. Only access from the audio thread.
pub struct AudioCallbackState {
    pub transport: Arc<TransportManager>,
    net_backend: UnsafeCell<Option<NetBackend>>,
    pub metering: Arc<MeteringManager>,
    pub sample_position: AtomicU64,
    pub sample_rate: f64,
}

unsafe impl Send for AudioCallbackState {}
unsafe impl Sync for AudioCallbackState {}

impl AudioCallbackState {
    pub fn new(
        transport: Arc<TransportManager>,
        metering: Arc<MeteringManager>,
        sample_rate: f64,
    ) -> Self {
        Self {
            transport,
            net_backend: UnsafeCell::new(None),
            metering,
            sample_position: AtomicU64::new(0),
            sample_rate,
        }
    }

    pub fn set_net_backend(&mut self, backend: NetBackend) {
        unsafe { *self.net_backend.get() = Some(backend) }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn net_backend_mut(&self) -> &mut Option<NetBackend> {
        &mut *self.net_backend.get()
    }
}

/// Process audio through the FunDSP Net. Output is interleaved stereo.
#[inline]
pub fn process_audio(state: &AudioCallbackState, output: &mut [f32]) {
    process_audio_inner(state, output);
}

#[inline]
fn process_audio_inner(state: &AudioCallbackState, output: &mut [f32]) {
    let frames = output.len() / 2;

    // Process any pending transport commands (from UI thread)
    state.transport.process_commands();

    // Advance transport
    if !state.transport.is_paused() {
        let beat_increment =
            frames as f64 / state.sample_rate * (state.transport.get_tempo() as f64 / 60.0);
        let _looped = state.transport.advance_position_rt(beat_increment);
    }

    // Process Net
    let net_backend = unsafe { state.net_backend_mut() };
    if let Some(ref mut backend) = net_backend {
        for i in 0..frames {
            let (l, r) = backend.get_stereo();
            output[i * 2] = l;
            output[i * 2 + 1] = r;
        }
    }

    // Update sample position
    state
        .sample_position
        .fetch_add(frames as u64, Ordering::Relaxed);

    // NOTE: LUFS metering is handled in the CPAL callback closure (output/core.rs)
    // to avoid heap allocations and Mutex locks on the RT thread.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_callback_state_creation() {
        let transport = Arc::new(TransportManager::new(44100.0));
        let metering = Arc::new(MeteringManager::new(44100.0));
        let state = AudioCallbackState::new(transport, metering, 44100.0);
        assert!(unsafe { (*state.net_backend.get()).is_none() });
    }

    #[test]
    fn test_process_audio_silence() {
        let transport = Arc::new(TransportManager::new(44100.0));
        let metering = Arc::new(MeteringManager::new(44100.0));
        let state = AudioCallbackState::new(transport, metering, 44100.0);
        let mut output = vec![0.0; 256];
        process_audio(&state, &mut output);
        assert!(output.iter().all(|&x| x == 0.0));
    }
}
