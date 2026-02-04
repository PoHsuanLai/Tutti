//! Unified MIDI system with port management, I/O, MPE, and MIDI 2.0.
//!
//! ## Quick Start
//!
//! ```ignore
//! use tutti_midi_io::{MidiSystem, MpeMode, MpeZoneConfig};
//!
//! // Create MIDI system with I/O and MPE
//! let midi = MidiSystem::builder()
//!     .io()
//!     .mpe(MpeMode::LowerZone(MpeZoneConfig::lower(15)))
//!     .build()?;
//!
//! // List and connect devices
//! let devices = midi.list_devices();
//! midi.connect_device_by_name("Keyboard")?;
//!
//! // Send MIDI messages
//! midi.send_note_on(0, 60, 100)?;
//! midi.send_cc(0, 74, 64)?;
//!
//! // Use MPE
//! let pitch = midi.mpe().pitch_bend(60);
//! let pressure = midi.mpe().pressure(60);
//!
//! // Create events for scheduling/sequencing
//! let event = midi.note_on(0, 60, 100);
//! ```

mod builder;

#[cfg(feature = "mpe")]
mod mpe_handle;

#[cfg(feature = "midi2")]
mod midi2_handle;

pub use builder::MidiSystemBuilder;

#[cfg(feature = "mpe")]
pub use mpe_handle::MpeHandle;

#[cfg(feature = "midi2")]
pub use midi2_handle::Midi2Handle;

use crate::error::Result;
use crate::event::MidiEvent;
use crate::port::{MidiPortManager, PortInfo};
use std::sync::Arc;

#[cfg(feature = "midi-io")]
use crate::error::Error;
#[cfg(feature = "midi-io")]
use crate::io::{MidiInputDevice, MidiInputManager, MidiOutputManager, MidiOutputMessage};

#[cfg(feature = "mpe")]
use crate::mpe::{MpeMode, MpeProcessor, PerNoteExpression};
#[cfg(feature = "mpe")]
use parking_lot::RwLock;

#[cfg(feature = "midi2")]
use crate::midi2::Midi2Event;

// ============================================================================
// MidiSystem - Main Entry Point
// ============================================================================

/// Complete MIDI system - the main entry point for tutti-midi
///
/// This owns all MIDI resources and provides a unified API.
/// Clone is cheap (Arc internally).
#[derive(Clone)]
pub struct MidiSystem {
    inner: Arc<MidiSystemInner>,
}

pub(crate) struct MidiSystemInner {
    pub(crate) port_manager: Arc<MidiPortManager>,
    #[cfg(feature = "midi-io")]
    pub(crate) input_manager: Option<Arc<MidiInputManager>>,
    #[cfg(feature = "midi-io")]
    pub(crate) output_manager: Option<Arc<MidiOutputManager>>,
    /// MPE processor with interior mutability for channel allocation
    ///
    /// Uses RwLock because:
    /// - Reading expression state (audio thread) is lock-free via atomics in PerNoteExpression
    /// - Channel allocation (outgoing MPE) requires mutation and is not time-critical
    #[cfg(feature = "mpe")]
    pub(crate) mpe_processor: Option<Arc<RwLock<MpeProcessor>>>,
    pub(crate) cc_manager: Option<Arc<crate::cc::CCMappingManager>>,
    pub(crate) output_collector: Option<Arc<crate::output_collector::MidiOutputAggregator>>,
}

impl MidiSystem {
    /// Create a new MIDI system builder
    ///
    /// # Example
    ///
    /// ```ignore
    /// let midi = MidiSystem::builder()
    ///     .io()
    ///     .build()?;
    /// ```
    pub fn builder() -> MidiSystemBuilder {
        MidiSystemBuilder::default()
    }

    // ==================== Port Management ====================

    /// Create a new MIDI input port
    ///
    /// Returns the port index for later reference.
    pub fn create_input_port(&self, name: impl Into<String>) -> usize {
        self.inner.port_manager.create_input_port(name)
    }

    /// Create a new MIDI output port
    ///
    /// Returns the port index for later reference.
    pub fn create_output_port(&self, name: impl Into<String>) -> usize {
        self.inner.port_manager.create_output_port(name)
    }

    /// Get information about a port
    pub fn port_info(&self, port_index: usize) -> Option<PortInfo> {
        self.inner.port_manager.get_port_info(port_index)
    }

    /// List all ports (input + output)
    pub fn list_ports(&self) -> Vec<PortInfo> {
        let mut ports = self.inner.port_manager.list_input_ports();
        ports.extend(self.inner.port_manager.list_output_ports());
        ports
    }

    /// List input ports only
    pub fn list_input_ports(&self) -> Vec<PortInfo> {
        self.inner.port_manager.list_input_ports()
    }

    /// List output ports only
    pub fn list_output_ports(&self) -> Vec<PortInfo> {
        self.inner.port_manager.list_output_ports()
    }

    // ==================== Hardware Device Connection ====================

    /// List available MIDI input devices
    #[cfg(feature = "midi-io")]
    pub fn list_devices(&self) -> Vec<MidiInputDevice> {
        MidiInputManager::list_devices()
    }

    /// Connect a hardware MIDI device by index
    ///
    /// Creates an input port automatically and connects to the device.
    #[cfg(feature = "midi-io")]
    pub fn connect_device(&self, device_index: usize) -> Result<()> {
        self.inner
            .input_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI I/O not enabled".to_string()))?
            .connect(device_index)
    }

    /// Connect a hardware MIDI device by name (partial match)
    #[cfg(feature = "midi-io")]
    pub fn connect_device_by_name(&self, name: &str) -> Result<()> {
        self.inner
            .input_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI I/O not enabled".to_string()))?
            .connect_by_name(name)
    }

    /// Disconnect the currently connected device
    #[cfg(feature = "midi-io")]
    pub fn disconnect_device(&self) {
        if let Some(ref manager) = self.inner.input_manager {
            manager.disconnect();
        }
    }

    /// Check if a device is connected
    #[cfg(feature = "midi-io")]
    pub fn is_device_connected(&self) -> bool {
        self.inner
            .input_manager
            .as_ref()
            .map(|m| m.is_connected())
            .unwrap_or(false)
    }

    /// Get the name of the connected device
    #[cfg(feature = "midi-io")]
    pub fn connected_device_name(&self) -> Option<String> {
        self.inner
            .input_manager
            .as_ref()
            .and_then(|m| m.connected_device_name())
    }

    /// Get the port index of the connected device
    #[cfg(feature = "midi-io")]
    pub fn connected_port_index(&self) -> Option<usize> {
        self.inner
            .input_manager
            .as_ref()
            .and_then(|m| m.connected_port_index())
    }

    // ==================== MIDI 1.0 Output ====================

    /// Send a Note On message
    ///
    /// * `channel` - MIDI channel (0-15)
    /// * `note` - MIDI note number (0-127)
    /// * `velocity` - Velocity (0-127, 0 = note off)
    #[cfg(feature = "midi-io")]
    pub fn send_note_on(&self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::note_on(channel, note, velocity));
        Ok(())
    }

    /// Send a Note Off message
    #[cfg(feature = "midi-io")]
    pub fn send_note_off(&self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::note_off(channel, note, velocity));
        Ok(())
    }

    /// Send a Control Change (CC) message
    ///
    /// * `channel` - MIDI channel (0-15)
    /// * `cc` - Controller number (0-127)
    /// * `value` - Controller value (0-127)
    #[cfg(feature = "midi-io")]
    pub fn send_cc(&self, channel: u8, cc: u8, value: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::control_change(channel, cc, value));
        Ok(())
    }

    /// Send a Pitch Bend message
    ///
    /// * `channel` - MIDI channel (0-15)
    /// * `value` - Pitch bend value (-8192 to 8191, 0 = center)
    #[cfg(feature = "midi-io")]
    pub fn send_pitch_bend(&self, channel: u8, value: i16) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::pitch_bend(channel, value));
        Ok(())
    }

    /// Send a Program Change message
    #[cfg(feature = "midi-io")]
    pub fn send_program_change(&self, channel: u8, program: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::program_change(channel, program));
        Ok(())
    }

    /// Send a MidiEvent directly
    #[cfg(feature = "midi-io")]
    pub fn send_event(&self, event: &MidiEvent) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::from_event(event));
        Ok(())
    }

    /// Fluent MIDI output builder.
    ///
    /// # Example
    /// ```ignore
    /// // Chain multiple messages
    /// midi.send()
    ///     .note_on(0, 60, 100)
    ///     .cc(0, 74, 64)
    ///     .pitch_bend(0, 0);
    /// ```
    pub fn send(&self) -> crate::midi_builder::MidiBuilder<'_> {
        crate::midi_builder::MidiBuilder::new(Some(self))
    }

    // ==================== MIDI 1.0 Event Creation ====================

    /// Create a Note On event (for scheduling, recording, etc.)
    ///
    /// * `channel` - MIDI channel (0-15)
    /// * `note` - MIDI note number (0-127)
    /// * `velocity` - Velocity (0-127)
    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_on(0, channel.min(15), note, velocity)
    }

    /// Create a Note On event with frame offset for sample-accurate timing
    pub fn note_on_at(
        &self,
        frame_offset: usize,
        channel: u8,
        note: u8,
        velocity: u8,
    ) -> MidiEvent {
        MidiEvent::note_on(frame_offset, channel.min(15), note, velocity)
    }

    /// Create a Note Off event
    pub fn note_off(&self, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_off(0, channel.min(15), note, velocity)
    }

    /// Create a Note Off event with frame offset
    pub fn note_off_at(
        &self,
        frame_offset: usize,
        channel: u8,
        note: u8,
        velocity: u8,
    ) -> MidiEvent {
        MidiEvent::note_off(frame_offset, channel.min(15), note, velocity)
    }

    /// Create a Control Change event
    pub fn cc(&self, channel: u8, cc: u8, value: u8) -> MidiEvent {
        MidiEvent::control_change(0, channel.min(15), cc, value)
    }

    /// Create a Control Change event with frame offset
    pub fn cc_at(&self, frame_offset: usize, channel: u8, cc: u8, value: u8) -> MidiEvent {
        MidiEvent::control_change(frame_offset, channel.min(15), cc, value)
    }

    /// Create a Pitch Bend event
    ///
    /// * `value` - 14-bit pitch bend (0-16383, 8192 = center)
    pub fn pitch_bend(&self, channel: u8, value: u16) -> MidiEvent {
        MidiEvent::pitch_bend(0, channel.min(15), value.min(16383))
    }

    /// Create a Pitch Bend event with frame offset
    pub fn pitch_bend_at(&self, frame_offset: usize, channel: u8, value: u16) -> MidiEvent {
        MidiEvent::pitch_bend(frame_offset, channel.min(15), value.min(16383))
    }

    /// Create a Channel Pressure (aftertouch) event
    pub fn channel_pressure(&self, channel: u8, pressure: u8) -> MidiEvent {
        MidiEvent::aftertouch(0, channel.min(15), pressure)
    }

    /// Create a Poly Pressure (per-note aftertouch) event
    pub fn poly_pressure(&self, channel: u8, note: u8, pressure: u8) -> MidiEvent {
        MidiEvent::poly_aftertouch(0, channel.min(15), note, pressure)
    }

    // ==================== MPE (MIDI Polyphonic Expression) ====================

    /// Get the MPE sub-handle for per-note expression
    #[cfg(feature = "mpe")]
    pub fn mpe(&self) -> MpeHandle {
        MpeHandle::new(self.inner.mpe_processor.clone())
    }

    /// Get the shared per-note expression state (for synth voices)
    #[cfg(feature = "mpe")]
    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.inner
            .mpe_processor
            .as_ref()
            .map(|p| p.read().expression())
    }

    /// Check if MPE is enabled
    #[cfg(feature = "mpe")]
    pub fn is_mpe_enabled(&self) -> bool {
        self.inner
            .mpe_processor
            .as_ref()
            .map(|p| !matches!(p.read().mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    // ==================== MIDI 2.0 ====================

    /// Get the MIDI 2.0 sub-handle for high-resolution messages
    #[cfg(feature = "midi2")]
    pub fn midi2(&self) -> Midi2Handle {
        Midi2Handle
    }

    /// Push a MIDI 2.0 event programmatically to a specific input port
    ///
    /// This allows sequencers, AI, tests, etc. to inject MIDI 2.0 events
    /// into the processing pipeline without hardware.
    ///
    /// * `port_index` - The input port index to push to
    /// * `event` - The MIDI 2.0 event to inject
    #[cfg(feature = "midi2")]
    pub fn push_midi2_event(&self, port_index: usize, event: Midi2Event) -> bool {
        let unified = crate::event::UnifiedMidiEvent::V2(event);
        self.inner
            .port_manager
            .push_unified_event(port_index, unified)
    }

    /// Push a unified MIDI event (1.0 or 2.0) programmatically to a specific input port
    #[cfg(feature = "midi2")]
    pub fn push_unified_event(
        &self,
        port_index: usize,
        event: crate::event::UnifiedMidiEvent,
    ) -> bool {
        self.inner
            .port_manager
            .push_unified_event(port_index, event)
    }

    // ==================== Advanced: Direct Access ====================

    /// Get direct access to the underlying port manager (advanced usage)
    ///
    /// Most users should use the high-level API methods instead.
    /// This is provided for framework integration scenarios.
    pub fn port_manager(&self) -> Arc<MidiPortManager> {
        self.inner.port_manager.clone()
    }

    /// Get direct access to the output manager (advanced usage)
    #[cfg(feature = "midi-io")]
    pub fn output_manager(&self) -> Option<Arc<MidiOutputManager>> {
        self.inner.output_manager.clone()
    }

    /// Get direct access to the CC mapping manager (advanced usage)
    pub fn cc_manager(&self) -> Option<Arc<crate::cc::CCMappingManager>> {
        self.inner.cc_manager.clone()
    }

    /// Get direct access to the MIDI output collector (advanced usage)
    pub fn output_collector(&self) -> Option<Arc<crate::output_collector::MidiOutputAggregator>> {
        self.inner.output_collector.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_system_creation() {
        let midi = MidiSystem::builder().build().unwrap();

        let port_id = midi.create_input_port("Test");
        assert_eq!(port_id, 0);

        let ports = midi.list_input_ports();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "Test");
    }

    #[test]
    fn test_event_creation() {
        let midi = MidiSystem::builder().build().unwrap();

        let event = midi.note_on(0, 60, 100);
        assert!(event.is_note_on());
        assert_eq!(event.note(), Some(60));
    }

    #[test]
    fn test_clone() {
        let midi = MidiSystem::builder().build().unwrap();
        let midi2 = midi.clone();

        midi.create_input_port("Port1");
        let ports = midi2.list_input_ports();
        assert_eq!(ports.len(), 1);
    }

    #[cfg(feature = "mpe")]
    #[test]
    fn test_mpe_builder() {
        use crate::mpe::MpeZoneConfig;

        let midi = MidiSystem::builder()
            .mpe(MpeMode::LowerZone(MpeZoneConfig::lower(15)))
            .build()
            .unwrap();

        assert!(midi.is_mpe_enabled());
        assert!(midi.mpe().has_lower_zone());
    }
}
