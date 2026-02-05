//! Real-time audio callback for FunDSP Net processing.

use crate::compat::{Arc, AtomicU64, Ordering, UnsafeCell};
use crate::metering::MeteringManager;
use crate::transport::{ClickNode, ClickState, TransportManager};
use fundsp::audionode::AudioNode;
use fundsp::audiounit::AudioUnit;
use fundsp::realnet::NetBackend;

#[cfg(feature = "midi")]
use crate::midi::{MidiInputSource, MidiRegistry, MidiRoutingSnapshot};
#[cfg(feature = "midi")]
use arc_swap::ArcSwap;

/// State for the real-time audio callback.
/// Uses `UnsafeCell` for interior mutability. Only access from the audio thread.
pub(crate) struct AudioCallbackState {
    pub(crate) transport: Arc<TransportManager>,
    net_backend: UnsafeCell<Option<NetBackend>>,
    pub(crate) metering: Arc<MeteringManager>,
    pub(crate) sample_position: AtomicU64,
    pub(crate) sample_rate: f64,

    /// Click node for metronome - mixed into output automatically
    click_node: UnsafeCell<Option<ClickNode>>,

    /// MIDI input source (hardware/virtual ports) - optional
    #[cfg(feature = "midi")]
    midi_input: Option<Arc<dyn MidiInputSource>>,

    /// MIDI registry for routing events to nodes - optional
    #[cfg(feature = "midi")]
    midi_registry: Option<MidiRegistry>,

    /// MIDI routing snapshot for RT-safe access to routing configuration.
    /// Uses ArcSwap for lock-free atomic updates from the main thread.
    #[cfg(feature = "midi")]
    midi_routing: Arc<ArcSwap<MidiRoutingSnapshot>>,
}

unsafe impl Send for AudioCallbackState {}
unsafe impl Sync for AudioCallbackState {}

impl AudioCallbackState {
    pub(crate) fn new(
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
            click_node: UnsafeCell::new(None),
            #[cfg(feature = "midi")]
            midi_input: None,
            #[cfg(feature = "midi")]
            midi_registry: None,
            #[cfg(feature = "midi")]
            midi_routing: Arc::new(ArcSwap::from_pointee(MidiRoutingSnapshot::empty())),
        }
    }

    /// Set the click node for metronome audio.
    pub(crate) fn set_click_node(&mut self, click_state: Arc<ClickState>, sample_rate: f64) {
        let node = ClickNode::new(click_state, sample_rate);
        unsafe { *self.click_node.get() = Some(node) }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn click_node_mut(&self) -> &mut Option<ClickNode> {
        &mut *self.click_node.get()
    }

    pub(crate) fn set_net_backend(&mut self, backend: NetBackend) {
        unsafe { *self.net_backend.get() = Some(backend) }
    }

    /// Set the MIDI input source for hardware MIDI routing.
    #[cfg(feature = "midi")]
    pub(crate) fn set_midi_input(&mut self, input: Arc<dyn MidiInputSource>) {
        self.midi_input = Some(input);
    }

    /// Set the MIDI registry for routing events to audio nodes.
    #[cfg(feature = "midi")]
    pub(crate) fn set_midi_registry(&mut self, registry: MidiRegistry) {
        self.midi_registry = Some(registry);
    }

    /// Set the MIDI routing snapshot Arc for RT-safe access.
    /// The MidiRoutingTable commits changes to this Arc.
    #[cfg(feature = "midi")]
    pub(crate) fn set_midi_routing(&mut self, routing: Arc<ArcSwap<MidiRoutingSnapshot>>) {
        self.midi_routing = routing;
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn net_backend_mut(&self) -> &mut Option<NetBackend> {
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

    // Process hardware MIDI input and route to audio nodes
    #[cfg(feature = "midi")]
    process_midi_input(state, frames);

    // Advance transport
    if !state.transport.is_paused() {
        let beat_increment =
            frames as f64 / state.sample_rate * (state.transport.get_tempo() as f64 / 60.0);
        let _looped = state.transport.advance_position_rt(beat_increment);
    }

    // Process Net and click node
    let net_backend = unsafe { state.net_backend_mut() };
    let click_node = unsafe { state.click_node_mut() };

    if let Some(ref mut backend) = net_backend {
        if state.transport.is_paused() {
            // Silence output when stopped
            for i in 0..frames {
                output[i * 2] = 0.0;
                output[i * 2 + 1] = 0.0;
            }
        } else {
            // Process audio and mix in metronome click
            for i in 0..frames {
                let (l, r) = backend.get_stereo();

                // Mix in click if present (click handles its own mode check)
                let (click_l, click_r) = if let Some(ref mut click) = click_node {
                    let frame = click.tick(&fundsp::prelude::Frame::default());
                    (frame[0], frame[1])
                } else {
                    (0.0, 0.0)
                };

                output[i * 2] = l + click_l;
                output[i * 2 + 1] = r + click_r;
            }
        }
    }

    // Update sample position
    state
        .sample_position
        .fetch_add(frames as u64, Ordering::Relaxed);

    // NOTE: LUFS metering is handled in the CPAL callback closure (output/core.rs)
    // to avoid heap allocations and Mutex locks on the RT thread.
}

/// Process MIDI input from hardware and route to the audio graph.
///
/// Uses the MidiRoutingSnapshot for channel/port/layer routing.
///
/// # RT Safety
/// This function is called from the audio thread and uses only lock-free operations:
/// - ArcSwap::load() for routing snapshot
/// - Bounded channel try_send() for registry queuing
#[cfg(feature = "midi")]
#[inline]
fn process_midi_input(state: &AudioCallbackState, frames: usize) {
    // Early return if no MIDI input or registry configured
    let (midi_input, midi_registry) = match (&state.midi_input, &state.midi_registry) {
        (Some(input), Some(registry)) => (input, registry),
        _ => return,
    };

    // Load routing snapshot (RT-safe, lock-free)
    let routing = state.midi_routing.load();

    if !routing.has_routes() {
        // No routing configured, discard MIDI input
        // Still call cycle_read to drain the buffer and prevent overflow
        let _ = midi_input.cycle_read(frames);
        return;
    }

    // Read all pending MIDI events from hardware ports (RT-safe)
    let events = midi_input.cycle_read(frames);

    if events.is_empty() {
        return;
    }

    // Route events through the routing table
    for &(port, event) in events {
        // Get all target units for this event
        for target in routing.route(port, &event) {
            midi_registry.queue(target, &[event]);
        }
    }
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
