//! Real-time audio callback for FunDSP Net processing.
//!
//! Uses sub-buffer splitting for sample-accurate MIDI timing and transport.
//! The CPAL buffer is split at event boundaries (MIDI frame offsets, loop wrap
//! points) and each segment is processed independently.

#[cfg(feature = "midi")]
use crate::compat::Vec;
use crate::compat::{Arc, AtomicU64, Ordering, UnsafeCell};
use crate::metering::MeteringManager;
use crate::transport::{
    ClickNode, ClickSettings, TransportClock, TransportHandle, TransportManager,
};
use fundsp::audionode::AudioNode;
use fundsp::audiounit::AudioUnit;
use fundsp::realnet::NetBackend;
use std::time::Instant;

#[cfg(feature = "midi")]
use crate::midi::{MidiInputSource, MidiRegistry, MidiRoutingSnapshot};
#[cfg(feature = "midi")]
use arc_swap::ArcSwap;
#[cfg(feature = "midi")]
use tutti_midi::MidiEvent;

/// Pre-allocated capacity for MIDI events per callback cycle.
#[cfg(feature = "midi")]
const MIDI_EVENT_BUFFER_CAPACITY: usize = 512;

/// Maximum number of split points per callback (MIDI events + loop boundary).
const MAX_SPLIT_POINTS: usize = 258;

/// State for the real-time audio callback.
/// Uses `UnsafeCell` for interior mutability. Only access from the audio thread.
pub(crate) struct AudioCallbackState {
    pub(crate) transport: Arc<TransportManager>,
    net_backend: UnsafeCell<Option<NetBackend>>,
    pub(crate) metering: Arc<MeteringManager>,
    pub(crate) sample_position: AtomicU64,
    #[allow(dead_code)]
    pub(crate) sample_rate: f64,

    /// Click node for metronome - mixed into output automatically
    click_node: UnsafeCell<Option<ClickNode>>,

    /// Transport clock - ticked per-sample for sample-accurate position
    transport_clock: UnsafeCell<Option<TransportClock>>,

    /// MIDI input source (hardware/virtual ports) - optional
    #[cfg(feature = "midi")]
    midi_input: Option<Arc<dyn MidiInputSource>>,

    /// MIDI registry for routing events to nodes - optional
    #[cfg(feature = "midi")]
    midi_registry: Option<MidiRegistry>,

    /// MIDI routing snapshot for RT-safe access to routing configuration.
    #[cfg(feature = "midi")]
    midi_routing: Arc<ArcSwap<MidiRoutingSnapshot>>,

    /// Pre-allocated buffer for sorted MIDI events: (frame_offset, port, event).
    /// Only accessed from the audio thread.
    #[cfg(feature = "midi")]
    midi_event_buffer: UnsafeCell<Vec<(usize, usize, MidiEvent)>>,
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
            transport_clock: UnsafeCell::new(None),
            #[cfg(feature = "midi")]
            midi_input: None,
            #[cfg(feature = "midi")]
            midi_registry: None,
            #[cfg(feature = "midi")]
            midi_routing: Arc::new(ArcSwap::from_pointee(MidiRoutingSnapshot::empty())),
            #[cfg(feature = "midi")]
            midi_event_buffer: UnsafeCell::new(Vec::with_capacity(MIDI_EVENT_BUFFER_CAPACITY)),
        }
    }

    pub(crate) fn set_click_node(
        &mut self,
        transport: TransportHandle,
        settings: Arc<ClickSettings>,
        sample_rate: f64,
    ) {
        let node = ClickNode::new(transport, settings, sample_rate);
        unsafe { *self.click_node.get() = Some(node) }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn click_node_mut(&self) -> &mut Option<ClickNode> {
        &mut *self.click_node.get()
    }

    pub(crate) fn set_transport_clock(&mut self, clock: TransportClock) {
        unsafe { *self.transport_clock.get() = Some(clock) }
    }

    #[inline]
    #[allow(clippy::mut_from_ref)]
    unsafe fn transport_clock_mut(&self) -> &mut Option<TransportClock> {
        &mut *self.transport_clock.get()
    }

    pub(crate) fn set_net_backend(&mut self, backend: NetBackend) {
        unsafe { *self.net_backend.get() = Some(backend) }
    }

    #[cfg(feature = "midi")]
    pub(crate) fn set_midi_input(&mut self, input: Arc<dyn MidiInputSource>) {
        self.midi_input = Some(input);
    }

    #[cfg(feature = "midi")]
    pub(crate) fn set_midi_registry(&mut self, registry: MidiRegistry) {
        self.midi_registry = Some(registry);
    }

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

#[inline]
pub(crate) fn process_audio(state: &AudioCallbackState, output: &mut [f32], buffer_start: Instant) {
    process_audio_inner(state, output, buffer_start);
}

/// Sample-accurate audio processing with sub-buffer splitting.
///
/// Splits the CPAL buffer at MIDI event boundaries, processing each segment
/// independently. TransportClock advances position per-sample (with loop
/// wrapping) and writes back to TransportManager's current_beat atomic.
///
/// This ensures:
/// - MIDI events take effect at their exact `frame_offset` within the buffer
/// - Transport position is sample-accurate (TransportClock ticked per-sample)
/// - Loop boundaries are handled at the exact sample by TransportClock
#[inline]
#[allow(unused_variables)]
fn process_audio_inner(state: &AudioCallbackState, output: &mut [f32], buffer_start: Instant) {
    let frames = output.len() / 2;

    // 1. Process pending transport commands (play/stop/locate from UI thread)
    state.transport.process_commands();

    // 2. Collect and sort MIDI events by frame_offset
    #[cfg(feature = "midi")]
    collect_midi_events_sorted(state, frames, buffer_start);

    // 3. Build sorted, deduped split points from MIDI offsets
    #[allow(unused_mut)]
    let mut split_points = [0usize; MAX_SPLIT_POINTS];
    #[allow(unused_mut)]
    let mut split_count = 0;

    #[cfg(feature = "midi")]
    {
        let buffer = unsafe { &*state.midi_event_buffer.get() };
        for &(offset, _, _) in buffer.iter() {
            if offset > 0
                && offset < frames
                && (split_count == 0 || split_points[split_count - 1] != offset)
            {
                split_points[split_count] = offset;
                split_count += 1;
                if split_count >= MAX_SPLIT_POINTS {
                    break;
                }
            }
        }
    }

    // 4. Process segments between split points
    let net_backend = unsafe { state.net_backend_mut() };
    let click_node = unsafe { state.click_node_mut() };
    let transport_clock = unsafe { state.transport_clock_mut() };
    let paused = state.transport.is_paused();

    let mut clock_output = [0.0f32];
    let mut segment_start = 0;
    let mut split_idx = 0;

    loop {
        let segment_end = if split_idx < split_count {
            split_points[split_idx]
        } else {
            frames
        };

        if segment_end > segment_start {
            let segment_frames = segment_end - segment_start;

            // 4a. Route MIDI events whose frame_offset falls in [segment_start, segment_end)
            #[cfg(feature = "midi")]
            route_midi_events_in_range(state, segment_start, segment_end);

            // 4b. Render per-sample: tick TransportClock, then graph, then click
            if let Some(ref mut backend) = net_backend {
                for i in 0..segment_frames {
                    // Tick transport clock BEFORE graph — updates current_beat atomic
                    // so graph nodes see per-sample-accurate position
                    if let Some(ref mut clock) = transport_clock {
                        clock.tick(&[], &mut clock_output);
                    }

                    let (l, r) = backend.get_stereo();

                    let (click_l, click_r) = if !paused {
                        if let Some(ref mut click) = click_node {
                            let frame = click.tick(&fundsp::prelude::Frame::default());
                            (frame[0], frame[1])
                        } else {
                            (0.0, 0.0)
                        }
                    } else {
                        (0.0, 0.0)
                    };

                    let out_idx = segment_start + i;
                    output[out_idx * 2] = l + click_l;
                    output[out_idx * 2 + 1] = r + click_r;
                }
            } else {
                // No graph backend — still tick the transport clock to keep position advancing
                if let Some(ref mut clock) = transport_clock {
                    for _ in 0..segment_frames {
                        clock.tick(&[], &mut clock_output);
                    }
                }
            }
        }

        if segment_end >= frames {
            break;
        }

        segment_start = segment_end;
        split_idx += 1;
    }

    state
        .sample_position
        .fetch_add(frames as u64, Ordering::Relaxed);
}

/// Collect MIDI events from hardware input, sorted by frame_offset.
///
/// Reads all pending events from `MidiInputSource::cycle_read()` and copies
/// them into the pre-allocated buffer sorted by `frame_offset` for sub-buffer
/// splitting.
#[cfg(feature = "midi")]
#[inline]
fn collect_midi_events_sorted(state: &AudioCallbackState, frames: usize, buffer_start: Instant) {
    let buffer = unsafe { &mut *state.midi_event_buffer.get() };
    buffer.clear();

    let midi_input = match &state.midi_input {
        Some(input) => input,
        None => return,
    };

    let routing = state.midi_routing.load();
    if !routing.has_routes() {
        // No routing configured — drain input to prevent overflow
        let _ = midi_input.cycle_read(frames, buffer_start, state.sample_rate);
        return;
    }

    let events = midi_input.cycle_read(frames, buffer_start, state.sample_rate);
    if events.is_empty() {
        return;
    }

    for &(port, event) in events {
        buffer.push((event.frame_offset, port, event));
    }

    // Stable sort preserves order for events at the same offset
    buffer.sort_by_key(|&(offset, _, _)| offset);
}

/// Route MIDI events whose `frame_offset` falls in `[start, end)` to the
/// registry for their target audio units.
#[cfg(feature = "midi")]
#[inline]
fn route_midi_events_in_range(state: &AudioCallbackState, start: usize, end: usize) {
    let buffer = unsafe { &*state.midi_event_buffer.get() };
    let midi_registry = match &state.midi_registry {
        Some(r) => r,
        None => return,
    };
    let routing = state.midi_routing.load();

    for &(offset, port, event) in buffer.iter() {
        if offset >= end {
            break; // Buffer is sorted by offset
        }
        if offset >= start {
            for target in routing.route(port, &event) {
                midi_registry.queue(target, &[event]);
            }
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
        process_audio(&state, &mut output, Instant::now());
        assert!(output.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_no_split_points_processes_full_buffer() {
        // With no MIDI events and no loop, the full buffer is one segment
        let transport = Arc::new(TransportManager::new(44100.0));
        let metering = Arc::new(MeteringManager::new(44100.0));
        let state = AudioCallbackState::new(transport, metering, 44100.0);
        let mut output = vec![0.0; 512]; // 256 frames stereo
        process_audio(&state, &mut output, Instant::now());
        // Should complete without panicking and output silence (no backend)
        assert!(output.iter().all(|&x| x == 0.0));
    }

    #[test]
    fn test_transport_advances_per_sample_with_clock() {
        // Verify that TransportClock advances position per-sample and writes
        // back to TransportManager's current_beat atomic
        let sample_rate = 44100.0;
        let transport = Arc::new(TransportManager::new(sample_rate));
        let metering = Arc::new(MeteringManager::new(sample_rate));
        let mut state = AudioCallbackState::new(transport, metering, sample_rate);

        // Create TransportClock with shared atomics
        let clock = TransportClock::new(
            state.transport.tempo().clone(),
            state.transport.paused().clone(),
            sample_rate,
        )
        .with_seek(
            state.transport.seek_target().clone(),
            state.transport.seek_pending().clone(),
        )
        .with_loop(
            state.transport.loop_enabled_flag().clone(),
            state.transport.loop_start_beat_atomic().clone(),
            state.transport.loop_end_beat_atomic().clone(),
        )
        .with_position_writeback(state.transport.current_beat().clone());
        state.set_transport_clock(clock);

        state.transport.set_paused(false);
        state.transport.set_current_beat(0.0);
        state.transport.set_tempo(120.0);

        let frames = 256;
        let mut output = vec![0.0f32; frames * 2];
        process_audio(&state, &mut output, Instant::now());

        // Expected: 256 samples at 120 BPM, 44100 Hz
        // beat_per_sample = (120/60) / 44100 = 2/44100
        // total = 256 * 2/44100 ≈ 0.01160998
        let expected_beat = 256.0 * (120.0 / 60.0) / sample_rate;
        let actual_beat = state.transport.get_current_beat();
        assert!(
            (actual_beat - expected_beat).abs() < 1e-6,
            "expected {expected_beat}, got {actual_beat}"
        );
    }

    #[test]
    fn test_transport_clock_loop_wrapping_in_callback() {
        // Verify TransportClock handles loop wrapping per-sample
        let sample_rate = 44100.0;
        let transport = Arc::new(TransportManager::new(sample_rate));
        let metering = Arc::new(MeteringManager::new(sample_rate));
        let mut state = AudioCallbackState::new(transport, metering, sample_rate);

        let clock = TransportClock::new(
            state.transport.tempo().clone(),
            state.transport.paused().clone(),
            sample_rate,
        )
        .with_seek(
            state.transport.seek_target().clone(),
            state.transport.seek_pending().clone(),
        )
        .with_loop(
            state.transport.loop_enabled_flag().clone(),
            state.transport.loop_start_beat_atomic().clone(),
            state.transport.loop_end_beat_atomic().clone(),
        )
        .with_position_writeback(state.transport.current_beat().clone());
        state.set_transport_clock(clock);

        state.transport.set_paused(false);
        state.transport.set_tempo(120.0);
        state.transport.set_loop_range(0.0, 4.0);
        state.transport.set_loop_enabled(true);
        state.transport.process_commands();
        // Start near the loop end
        state.transport.set_current_beat(3.99);
        // Also seek the clock to match
        state.transport.seek_target().set(3.99);
        state.transport.seek_pending().set(true);

        // Process a large buffer that will cross the loop boundary
        let frames = 1024;
        let mut output = vec![0.0f32; frames * 2];
        process_audio(&state, &mut output, Instant::now());

        // After crossing beat 4.0, position should have wrapped back near 0
        let beat = state.transport.get_current_beat();
        assert!(beat < 4.0, "expected beat wrapped below 4.0, got {beat}");
    }
}
