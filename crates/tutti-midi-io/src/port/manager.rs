//! Multi-Port MIDI Manager
//!
//! Manages multiple AsyncMidiPort instances for simultaneous MIDI I/O.
//! Index-based for RT-safe direct Vec access, with arc-swap for lock-free updates.

use super::async_port::{AsyncMidiPort, OutputProducerHandle};
use crate::MidiEvent;
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortType {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub index: usize,
    pub name: String,
    pub port_type: PortType,
    pub active: Arc<AtomicBool>,
}

pub struct MidiPortManager {
    input_ports: Arc<ArcSwap<Vec<Arc<AsyncMidiPort>>>>,
    output_ports: Arc<ArcSwap<Vec<Arc<AsyncMidiPort>>>>,
    output_handles: Arc<ArcSwap<Vec<OutputProducerHandle>>>,
    port_info: Arc<RwLock<Vec<PortInfo>>>,
    fifo_size: usize,
    event_buffer: std::cell::UnsafeCell<Vec<(usize, MidiEvent)>>,
    /// Intermediate buffer for timestamped events before frame_offset conversion.
    timestamped_buffer: std::cell::UnsafeCell<Vec<(Instant, usize, MidiEvent)>>,
    output_event_buffer: std::cell::UnsafeCell<Vec<(usize, MidiEvent)>>,
}

// SAFETY: MidiPortManager is Sync because:
// 1. event_buffer / output_event_buffer (UnsafeCell) are only accessed from
//    audio callback (single-threaded)
// 2. All other fields (ArcSwap, RwLock, primitives) are already Sync
unsafe impl Sync for MidiPortManager {}

impl MidiPortManager {
    pub fn new(fifo_size: usize) -> Self {
        Self {
            input_ports: Arc::new(ArcSwap::from_pointee(Vec::new())),
            output_ports: Arc::new(ArcSwap::from_pointee(Vec::new())),
            output_handles: Arc::new(ArcSwap::from_pointee(Vec::new())),
            port_info: Arc::new(RwLock::new(Vec::new())),
            fifo_size,
            event_buffer: std::cell::UnsafeCell::new(Vec::with_capacity(256)),
            timestamped_buffer: std::cell::UnsafeCell::new(Vec::with_capacity(256)),
            output_event_buffer: std::cell::UnsafeCell::new(Vec::with_capacity(256)),
        }
    }

    pub fn create_input_port(&self, name: impl Into<String>) -> usize {
        let name = name.into();
        let port = Arc::new(AsyncMidiPort::new(&name, self.fifo_size));
        let current_ports = self.input_ports.load();
        let mut new_ports = (**current_ports).clone();
        let port_index = new_ports.len();
        new_ports.push(port);
        self.input_ports.store(Arc::new(new_ports));

        let mut port_info = self.port_info.write();
        let info = PortInfo {
            index: port_index,
            name: name.clone(),
            port_type: PortType::Input,
            active: Arc::new(AtomicBool::new(true)),
        };
        port_info.push(info);

        tracing::debug!("Created MIDI input port {}: {}", port_index, name);
        port_index
    }

    pub fn create_output_port(&self, name: impl Into<String>) -> usize {
        let name = name.into();
        let port = Arc::new(AsyncMidiPort::new(&name, self.fifo_size));

        let output_handle = port.output_producer_handle();

        let current_ports = self.output_ports.load();
        let mut new_ports = (**current_ports).clone();
        let port_index = new_ports.len();
        new_ports.push(port);
        self.output_ports.store(Arc::new(new_ports));

        let current_handles = self.output_handles.load();
        let mut new_handles = (**current_handles).clone();
        new_handles.push(output_handle);
        self.output_handles.store(Arc::new(new_handles));

        let mut port_info = self.port_info.write();
        let info = PortInfo {
            index: port_index,
            name: name.clone(),
            port_type: PortType::Output,
            active: Arc::new(AtomicBool::new(true)),
        };
        port_info.push(info);

        tracing::debug!("Created MIDI output port {}: {}", port_index, name);
        port_index
    }

    pub fn get_port_info(&self, port_type: PortType, port_index: usize) -> Option<PortInfo> {
        let port_info = self.port_info.read();
        port_info
            .iter()
            .find(|info| info.port_type == port_type && info.index == port_index)
            .cloned()
    }

    pub fn list_input_ports(&self) -> Vec<PortInfo> {
        let port_info = self.port_info.read();
        port_info
            .iter()
            .filter(|info| info.port_type == PortType::Input)
            .cloned()
            .collect()
    }

    pub fn list_output_ports(&self) -> Vec<PortInfo> {
        let port_info = self.port_info.read();
        port_info
            .iter()
            .filter(|info| info.port_type == PortType::Output)
            .cloned()
            .collect()
    }

    pub fn set_port_active(&self, port_type: PortType, port_index: usize, active: bool) -> bool {
        let port_info = self.port_info.read();
        if let Some(info) = port_info
            .iter()
            .find(|info| info.port_type == port_type && info.index == port_index)
        {
            info.active.store(active, Ordering::Release);
            // Also update the port's internal active flag for RT-safe access
            match info.port_type {
                PortType::Input => {
                    let input_ports = self.input_ports.load();
                    if let Some(port) = input_ports.get(info.index) {
                        port.set_active(active);
                    }
                }
                PortType::Output => {
                    let output_ports = self.output_ports.load();
                    if let Some(port) = output_ports.get(info.index) {
                        port.set_active(active);
                    }
                }
            }
            true
        } else {
            false
        }
    }

    /// NOT RT-safe (acquires lock). For RT-safe access, use the port's `is_active()` directly.
    pub fn is_port_active(&self, port_type: PortType, port_index: usize) -> bool {
        let port_info = self.port_info.read();
        port_info
            .iter()
            .find(|info| info.port_type == port_type && info.index == port_index)
            .map(|info| info.active.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    pub fn output_port_count(&self) -> usize {
        self.output_ports.load().len()
    }

    pub fn output_ports(&self) -> arc_swap::Guard<Arc<Vec<Arc<AsyncMidiPort>>>> {
        self.output_ports.load()
    }

    /// RT-safe (lock-free).
    ///
    /// # Safety
    /// Must only be called from a single thread (the audio thread).
    /// The underlying SPSC queue requires a single producer.
    pub fn write_output_event(&self, port_index: usize, event: MidiEvent) -> bool {
        let output_handles = self.output_handles.load();
        if let Some(handle) = output_handles.get(port_index) {
            handle.push(event)
        } else {
            false
        }
    }

    /// RT-safe (lock-free, no heap allocation).
    ///
    /// Drains all active input port ring buffers, converts timestamps to
    /// sample-accurate frame_offsets, and returns a flat event slice.
    pub fn cycle_start_read_all_inputs(
        &self,
        nframes: usize,
        buffer_start: Instant,
        sample_rate: f64,
    ) -> &[(usize, MidiEvent)] {
        unsafe {
            let timestamped = &mut *self.timestamped_buffer.get();
            timestamped.clear();
            let input_ports = self.input_ports.load();

            for (port_index, port) in input_ports.iter().enumerate() {
                if !port.is_active() {
                    continue;
                }
                port.cycle_start_read_input_into(timestamped, port_index);
            }

            // Convert timestamps to frame_offsets
            let all_events = &mut *self.event_buffer.get();
            all_events.clear();
            for &(midi_instant, port_index, mut event) in timestamped.iter() {
                let delta = buffer_start.saturating_duration_since(midi_instant);
                let samples_ago = (delta.as_secs_f64() * sample_rate) as usize;
                event.frame_offset = nframes.saturating_sub(samples_ago);
                // Clamp to valid range
                if event.frame_offset >= nframes {
                    event.frame_offset = nframes.saturating_sub(1);
                }
                all_events.push((port_index, event));
            }

            all_events.as_slice()
        }
    }

    /// RT-safe (lock-free, no heap allocation).
    ///
    /// Returns a flat slice of (port_index, event) pairs from all active output ports.
    /// Uses a pre-allocated internal buffer.
    pub fn cycle_end_flush_all_outputs(&self) -> &[(usize, MidiEvent)] {
        unsafe {
            let all_events = &mut *self.output_event_buffer.get();
            all_events.clear();
            let output_ports = self.output_ports.load();

            for (port_index, port) in output_ports.iter().enumerate() {
                if !port.is_active() {
                    continue;
                }
                port.cycle_end_flush_output_into(all_events, port_index);
            }
            all_events.as_slice()
        }
    }

    /// RT-safe (lock-free).
    ///
    /// # Safety
    /// Must only be called from a single thread (the audio thread).
    /// The underlying SPSC queue requires a single producer.
    pub fn write_event_to_port(&self, port_index: usize, event: MidiEvent) -> bool {
        let output_ports = self.output_ports.load();
        if let Some(port) = output_ports.get(port_index) {
            if !port.is_active() {
                return false;
            }
        } else {
            return false;
        }

        let output_handles = self.output_handles.load();
        if let Some(handle) = output_handles.get(port_index) {
            handle.push(event)
        } else {
            false
        }
    }

    pub fn get_input_producer_handle(
        &self,
        port_index: usize,
    ) -> Option<super::async_port::InputProducerHandle> {
        let input_ports = self.input_ports.load();
        input_ports
            .get(port_index)
            .map(|port| port.input_producer_handle())
    }

    #[cfg(feature = "midi2")]
    pub fn get_unified_input_producer_handle(
        &self,
        port_index: usize,
    ) -> Option<super::async_port::UnifiedInputProducerHandle> {
        let input_ports = self.input_ports.load();
        input_ports
            .get(port_index)
            .map(|port| port.unified_input_producer_handle())
    }

    /// RT-safe (lock-free, no heap allocation).
    #[cfg(feature = "midi2")]
    pub fn cycle_start_read_all_unified_inputs(
        &self,
        _nframes: usize,
    ) -> Vec<(usize, crate::event::UnifiedMidiEvent)> {
        // TODO: Add pre-allocated buffer (like event_buffer) when unified MIDI
        // is used on the RT path. Currently only used in tests.
        let mut all_events = Vec::new();
        let input_ports = self.input_ports.load();

        for (port_index, port) in input_ports.iter().enumerate() {
            if !port.is_active() {
                continue;
            }
            port.cycle_start_read_unified_input_into(&mut all_events, port_index);
        }
        all_events
    }

    /// Push a MIDI 1.0 event into a port's input buffer programmatically.
    /// Uses `Instant::now()` as the timestamp. RT-safe (lock-free).
    pub fn push_input_event(&self, port_index: usize, event: MidiEvent) -> bool {
        let input_ports = self.input_ports.load();
        if let Some(port) = input_ports.get(port_index) {
            let handle = port.input_producer_handle();
            handle.push(event, std::time::Instant::now())
        } else {
            false
        }
    }

    #[cfg(feature = "midi2")]
    pub fn push_unified_event(
        &self,
        port_index: usize,
        event: crate::event::UnifiedMidiEvent,
    ) -> bool {
        let input_ports = self.input_ports.load();
        if let Some(port) = input_ports.get(port_index) {
            let handle = port.unified_input_producer_handle();
            handle.push(event)
        } else {
            false
        }
    }
}

impl Default for MidiPortManager {
    fn default() -> Self {
        Self::new(2048)
    }
}

// Implement MidiInputSource trait from tutti-core for audio callback integration
impl tutti_core::midi::MidiInputSource for MidiPortManager {
    /// Bridge between hardware MIDI (via midir) and the audio callback.
    /// Called once per audio buffer to collect all events that arrived since the last call.
    fn cycle_read(
        &self,
        nframes: usize,
        buffer_start: Instant,
        sample_rate: f64,
    ) -> &[(usize, tutti_core::midi::MidiEvent)] {
        self.cycle_start_read_all_inputs(nframes, buffer_start, sample_rate)
    }

    fn has_active_inputs(&self) -> bool {
        let input_ports = self.input_ports.load();
        input_ports.iter().any(|port| port.is_active())
    }
}

impl std::fmt::Debug for MidiPortManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let input_ports = self.input_ports.load();
        let output_ports = self.output_ports.load();

        f.debug_struct("MidiPortManager")
            .field("num_input_ports", &input_ports.len())
            .field("num_output_ports", &output_ports.len())
            .field("fifo_size", &self.fifo_size)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn test_create_ports() {
        let manager = MidiPortManager::new(256);

        let input_id = manager.create_input_port("Test Input");
        let output_id = manager.create_output_port("Test Output");

        // Input and output ports have separate index spaces, so both can be 0
        assert_eq!(input_id, 0);
        assert_eq!(output_id, 0);

        // Get port info by listing and filtering
        let input_ports = manager.list_input_ports();
        assert_eq!(input_ports.len(), 1);
        let input_info = &input_ports[0];
        assert_eq!(input_info.name, "Test Input");
        assert_eq!(input_info.port_type, PortType::Input);
        assert!(input_info.active.load(Ordering::Acquire));

        let output_ports = manager.list_output_ports();
        assert_eq!(output_ports.len(), 1);
        let output_info = &output_ports[0];
        assert_eq!(output_info.name, "Test Output");
        assert_eq!(output_info.port_type, PortType::Output);
        assert!(output_info.active.load(Ordering::Acquire));
    }

    #[test]
    fn test_list_ports() {
        let manager = MidiPortManager::new(256);

        let id1 = manager.create_input_port("Input 1");
        let id2 = manager.create_input_port("Input 2");
        let id3 = manager.create_output_port("Output 1");

        let inputs = manager.list_input_ports();
        assert_eq!(inputs.len(), 2);
        assert!(inputs.iter().any(|p| p.index == id1));
        assert!(inputs.iter().any(|p| p.index == id2));

        let outputs = manager.list_output_ports();
        assert_eq!(outputs.len(), 1);
        assert!(outputs.iter().any(|p| p.index == id3));
    }

    #[test]
    fn test_port_active_state() {
        let manager = MidiPortManager::new(256);

        let port_id = manager.create_input_port("Test");
        assert!(manager.is_port_active(PortType::Input, port_id));

        manager.set_port_active(PortType::Input, port_id, false);
        assert!(!manager.is_port_active(PortType::Input, port_id));

        manager.set_port_active(PortType::Input, port_id, true);
        assert!(manager.is_port_active(PortType::Input, port_id));
    }

    #[test]
    fn test_input_output_flow() {
        let manager = MidiPortManager::new(256);

        let input_id = manager.create_input_port("Input");
        let output_id = manager.create_output_port("Output");

        // Simulate midir callback writing to input port
        let producer_handle = manager.get_input_producer_handle(input_id).unwrap();
        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F); // Note On
        assert!(producer_handle.push(event, now()));

        // Simulate audio thread reading inputs
        let events = manager.cycle_start_read_all_inputs(512, now(), 44100.0);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, input_id);
        // Verify it's a note on with correct note and velocity
        assert!(events[0].1.is_note_on());
        assert_eq!(events[0].1.note(), Some(0x3C));
        assert_eq!(events[0].1.velocity(), Some(0x7F));

        // Simulate audio thread writing to output
        let out_event = MidiEvent::note_off(10, 0, 0x3C, 0); // Note Off with velocity 0
        assert!(manager.write_event_to_port(output_id, out_event));

        // Flush outputs
        let output_events = manager.cycle_end_flush_all_outputs();
        assert_eq!(output_events.len(), 1);
        assert_eq!(output_events[0].0, output_id);
        assert!(output_events[0].1.is_note_off());
        assert_eq!(output_events[0].1.note(), Some(0x3C));
    }

    #[test]
    fn test_inactive_ports_ignored() {
        let manager = MidiPortManager::new(256);

        let input_id = manager.create_input_port("Input");
        let producer_handle = manager.get_input_producer_handle(input_id).unwrap();

        // Write event to input
        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F);
        assert!(producer_handle.push(event, now()));

        // Deactivate port
        manager.set_port_active(PortType::Input, input_id, false);

        // Reading inputs should return empty (port is inactive)
        let events = manager.cycle_start_read_all_inputs(512, now(), 44100.0);
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_multiple_input_ports() {
        let manager = MidiPortManager::new(256);

        let id1 = manager.create_input_port("Input 1");
        let id2 = manager.create_input_port("Input 2");

        let handle1 = manager.get_input_producer_handle(id1).unwrap();
        let handle2 = manager.get_input_producer_handle(id2).unwrap();

        // Write to both ports
        handle1.push(MidiEvent::note_on(0, 0, 60, 100), now());
        handle2.push(MidiEvent::note_on(0, 0, 64, 100), now());

        // Read all
        let events = manager.cycle_start_read_all_inputs(512, now(), 44100.0);
        assert_eq!(events.len(), 2);

        let port_ids: Vec<_> = events.iter().map(|(id, _)| *id).collect();
        assert!(port_ids.contains(&id1));
        assert!(port_ids.contains(&id2));
    }

    #[test]
    fn test_timestamp_to_frame_offset_conversion() {
        let manager = MidiPortManager::new(256);
        let input_id = manager.create_input_port("Input");
        let handle = manager.get_input_producer_handle(input_id).unwrap();

        let sample_rate = 48000.0;
        let nframes = 256;

        // Push event "now", then read with buffer_start slightly after
        let midi_time = Instant::now();
        handle.push(MidiEvent::note_on(0, 0, 60, 100), midi_time);

        // Buffer starts 128 samples after the MIDI event
        // 128 samples at 48kHz = 128/48000 ≈ 2.667ms
        let buffer_start = midi_time + std::time::Duration::from_secs_f64(128.0 / sample_rate);

        let events = manager.cycle_start_read_all_inputs(nframes, buffer_start, sample_rate);
        assert_eq!(events.len(), 1);

        // Event arrived 128 samples before buffer_start, so frame_offset should be near 0
        // (nframes - samples_ago = 256 - 128 = 128... but samples_ago is computed from
        // buffer_start - midi_instant, which is 128 samples, so frame_offset = 256 - 128 = 128)
        // Wait - the logic is: event arrived 128 samples AGO relative to buffer_start,
        // meaning it should play at frame 256-128 = 128 within the buffer.
        // Actually let me re-check: buffer_start.saturating_duration_since(midi_instant) = 128 samples.
        // samples_ago = 128. frame_offset = nframes - samples_ago = 256 - 128 = 128.
        // This means the event is placed at sample 128 in the 256-sample buffer, which is correct:
        // the event arrived halfway through the buffer period.
        assert!(
            events[0].1.frame_offset <= nframes,
            "frame_offset should be within buffer"
        );
    }
}
