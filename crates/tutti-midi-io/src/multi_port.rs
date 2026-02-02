//! Multi-Port MIDI Manager
//!
//! Manages multiple AsyncMidiPort instances for simultaneous MIDI I/O.
//! Index-based for RT-safe direct Vec access, with arc-swap for lock-free updates.

use super::async_port::{AsyncMidiPort, MidiEvent};
use arc_swap::ArcSwap;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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

/// Multi-port MIDI manager with lock-free arc-swap pattern for RT-safe updates.
pub struct MidiPortManager {
    input_ports: Arc<ArcSwap<Vec<Arc<AsyncMidiPort>>>>,
    output_ports: Arc<ArcSwap<Vec<Arc<AsyncMidiPort>>>>,
    port_info: Arc<RwLock<Vec<PortInfo>>>,
    fifo_size: usize,
    event_buffer: std::cell::UnsafeCell<Vec<(usize, MidiEvent)>>,
}

// SAFETY: MidiPortManager is Sync because:
// 1. event_buffer (UnsafeCell) is only accessed from audio callback (single-threaded)
// 2. All other fields (ArcSwap, RwLock, primitives) are already Sync
// 3. We never access event_buffer from multiple threads concurrently
unsafe impl Sync for MidiPortManager {}

impl MidiPortManager {
    pub fn new(fifo_size: usize) -> Self {
        Self {
            input_ports: Arc::new(ArcSwap::from_pointee(Vec::new())),
            output_ports: Arc::new(ArcSwap::from_pointee(Vec::new())),
            port_info: Arc::new(RwLock::new(Vec::new())),
            fifo_size,
            event_buffer: std::cell::UnsafeCell::new(Vec::with_capacity(256)),
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
        let current_ports = self.output_ports.load();
        let mut new_ports = (**current_ports).clone();
        let port_index = new_ports.len();
        new_ports.push(port);
        self.output_ports.store(Arc::new(new_ports));

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

    pub fn get_port_info(&self, port_index: usize) -> Option<PortInfo> {
        let port_info = self.port_info.read();
        port_info.get(port_index).cloned()
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

    pub fn set_port_active(&self, port_index: usize, active: bool) -> bool {
        // Update both input and output ports (index is shared space in port_info)
        let port_info = self.port_info.read();
        if let Some(info) = port_info.get(port_index) {
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

    /// Check if port is active. NOT RT-safe (acquires lock).
    /// For RT-safe access, use the port's is_active() method directly.
    pub fn is_port_active(&self, port_index: usize) -> bool {
        let port_info = self.port_info.read();
        port_info
            .get(port_index)
            .map(|info| info.active.load(Ordering::Acquire))
            .unwrap_or(false)
    }

    pub fn output_port_count(&self) -> usize {
        self.output_ports.load().len()
    }

    pub fn output_ports(&self) -> arc_swap::Guard<Arc<Vec<Arc<AsyncMidiPort>>>> {
        self.output_ports.load()
    }

    pub fn write_output_event(&self, port_index: usize, event: MidiEvent) -> bool {
        let output_ports = self.output_ports.load();
        if let Some(port) = output_ports.get(port_index) {
            port.write_event(event)
        } else {
            false
        }
    }

    // ==================== RT Thread Methods ====================

    /// Read input from all active ports. RT-safe (lock-free).
    pub fn cycle_start_read_all_inputs(&self, nframes: usize) -> &[(usize, MidiEvent)] {
        unsafe {
            let all_events = &mut *self.event_buffer.get();
            all_events.clear();
            let input_ports = self.input_ports.load();

            for (port_index, port) in input_ports.iter().enumerate() {
                // Use port's atomic is_active() for RT-safe check (no lock)
                if !port.is_active() {
                    continue;
                }
                let events = port.cycle_start_read_input(nframes);
                for event in events {
                    all_events.push((port_index, event));
                }
            }
            all_events.as_slice()
        }
    }

    /// Flush output from all active ports. RT-safe (lock-free).
    pub fn cycle_end_flush_all_outputs(&self) -> Vec<(usize, Vec<MidiEvent>)> {
        let mut all_output_events = Vec::new();
        let output_ports = self.output_ports.load();

        for (port_index, port) in output_ports.iter().enumerate() {
            // Use port's atomic is_active() for RT-safe check (no lock)
            if !port.is_active() {
                continue;
            }
            let events = port.cycle_end_flush_output();
            if !events.is_empty() {
                all_output_events.push((port_index, events));
            }
        }
        all_output_events
    }

    pub fn write_event_to_port(&self, port_index: usize, event: MidiEvent) -> bool {
        let output_ports = self.output_ports.load();
        if let Some(port) = output_ports.get(port_index) {
            // Use port's atomic is_active() for RT-safe check (no lock)
            if !port.is_active() {
                return false;
            }
            port.write_event(event)
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

    /// Get a lock-free unified input producer handle for a port.
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

    /// Read unified input from all active ports. RT-safe (lock-free).
    #[cfg(feature = "midi2")]
    pub fn cycle_start_read_all_unified_inputs(
        &self,
        _nframes: usize,
    ) -> Vec<(usize, crate::event::UnifiedMidiEvent)> {
        let mut all_events = Vec::new();
        let input_ports = self.input_ports.load();

        for (port_index, port) in input_ports.iter().enumerate() {
            if !port.is_active() {
                continue;
            }
            let events = port.cycle_start_read_unified_input();
            for event in events {
                all_events.push((port_index, event));
            }
        }
        all_events
    }

    /// Push a unified MIDI event to a specific input port.
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

    // NOTE: This test is commented out because remove_port() method
    // doesn't exist in the current API. Ports can be deactivated with set_port_active().
    // #[test]
    // fn test_remove_port() { ... }

    #[test]
    fn test_port_active_state() {
        let manager = MidiPortManager::new(256);

        let port_id = manager.create_input_port("Test");
        assert!(manager.is_port_active(port_id));

        manager.set_port_active(port_id, false);
        assert!(!manager.is_port_active(port_id));

        manager.set_port_active(port_id, true);
        assert!(manager.is_port_active(port_id));
    }

    #[test]
    fn test_input_output_flow() {
        let manager = MidiPortManager::new(256);

        let input_id = manager.create_input_port("Input");
        let output_id = manager.create_output_port("Output");

        // Simulate midir callback writing to input port
        let producer_handle = manager.get_input_producer_handle(input_id).unwrap();
        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F); // Note On
        assert!(producer_handle.push(event));

        // Simulate audio thread reading inputs
        let events = manager.cycle_start_read_all_inputs(512);
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
        assert_eq!(output_events[0].1.len(), 1);
        // Verify it's a note off with correct note
        assert!(output_events[0].1[0].is_note_off());
        assert_eq!(output_events[0].1[0].note(), Some(0x3C));
    }

    #[test]
    fn test_inactive_ports_ignored() {
        let manager = MidiPortManager::new(256);

        let input_id = manager.create_input_port("Input");
        let producer_handle = manager.get_input_producer_handle(input_id).unwrap();

        // Write event to input
        let event = MidiEvent::note_on(0, 0, 0x3C, 0x7F);
        assert!(producer_handle.push(event));

        // Deactivate port
        manager.set_port_active(input_id, false);

        // Reading inputs should return empty (port is inactive)
        let events = manager.cycle_start_read_all_inputs(512);
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
        handle1.push(MidiEvent::note_on(0, 0, 60, 100));
        handle2.push(MidiEvent::note_on(0, 0, 64, 100));

        // Read all
        let events = manager.cycle_start_read_all_inputs(512);
        assert_eq!(events.len(), 2);

        let port_ids: Vec<_> = events.iter().map(|(id, _)| *id).collect();
        assert!(port_ids.contains(&id1));
        assert!(port_ids.contains(&id2));
    }
}
