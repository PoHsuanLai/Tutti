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

#[cfg(feature = "midi-io")]
use crate::error::Result;
use crate::event::MidiEvent;
use crate::port::{MidiPortManager, PortInfo, PortType};
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

/// Complete MIDI system - the main entry point for tutti-midi.
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
    /// RwLock because reads (expression state) use atomics internally,
    /// and writes (channel allocation) are not time-critical.
    #[cfg(feature = "mpe")]
    pub(crate) mpe_processor: Option<Arc<RwLock<MpeProcessor>>>,
    pub(crate) cc_manager: Option<Arc<crate::cc::CCMappingManager>>,
    pub(crate) output_collector: Option<Arc<crate::output_collector::MidiOutputAggregator>>,
}

impl MidiSystem {
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

    pub fn create_input_port(&self, name: impl Into<String>) -> usize {
        self.inner.port_manager.create_input_port(name)
    }

    pub fn create_output_port(&self, name: impl Into<String>) -> usize {
        self.inner.port_manager.create_output_port(name)
    }

    pub fn port_info(&self, port_type: PortType, port_index: usize) -> Option<PortInfo> {
        self.inner.port_manager.get_port_info(port_type, port_index)
    }

    /// Returns both input and output ports.
    pub fn list_ports(&self) -> Vec<PortInfo> {
        let mut ports = self.inner.port_manager.list_input_ports();
        ports.extend(self.inner.port_manager.list_output_ports());
        ports
    }

    pub fn list_input_ports(&self) -> Vec<PortInfo> {
        self.inner.port_manager.list_input_ports()
    }

    pub fn list_output_ports(&self) -> Vec<PortInfo> {
        self.inner.port_manager.list_output_ports()
    }

    #[cfg(feature = "midi-io")]
    pub fn list_devices(&self) -> Vec<MidiInputDevice> {
        MidiInputManager::list_devices()
    }

    /// Creates an input port automatically and connects to the device.
    #[cfg(feature = "midi-io")]
    pub fn connect_device(&self, device_index: usize) -> Result<()> {
        self.inner
            .input_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI I/O not enabled".to_string()))?
            .connect(device_index)
    }

    /// Connects by partial name match.
    #[cfg(feature = "midi-io")]
    pub fn connect_device_by_name(&self, name: &str) -> Result<()> {
        self.inner
            .input_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI I/O not enabled".to_string()))?
            .connect_by_name(name)
    }

    #[cfg(feature = "midi-io")]
    pub fn disconnect_device(&self) {
        if let Some(ref manager) = self.inner.input_manager {
            manager.disconnect();
        }
    }

    #[cfg(feature = "midi-io")]
    pub fn is_device_connected(&self) -> bool {
        self.inner
            .input_manager
            .as_ref()
            .map(|m| m.is_connected())
            .unwrap_or(false)
    }

    #[cfg(feature = "midi-io")]
    pub fn connected_device_name(&self) -> Option<String> {
        self.inner
            .input_manager
            .as_ref()
            .and_then(|m| m.connected_device_name())
    }

    #[cfg(feature = "midi-io")]
    pub fn connected_port_index(&self) -> Option<usize> {
        self.inner
            .input_manager
            .as_ref()
            .and_then(|m| m.connected_port_index())
    }

    /// `channel` 0-15, `note` 0-127, `velocity` 0-127 (0 = note off).
    #[cfg(feature = "midi-io")]
    pub fn send_note_on(&self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::note_on(channel, note, velocity));
        Ok(())
    }

    #[cfg(feature = "midi-io")]
    pub fn send_note_off(&self, channel: u8, note: u8, velocity: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::note_off(channel, note, velocity));
        Ok(())
    }

    /// `channel` 0-15, `cc` 0-127, `value` 0-127.
    #[cfg(feature = "midi-io")]
    pub fn send_cc(&self, channel: u8, cc: u8, value: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::control_change(channel, cc, value));
        Ok(())
    }

    /// `value` range: -8192 to 8191 (0 = center).
    #[cfg(feature = "midi-io")]
    pub fn send_pitch_bend(&self, channel: u8, value: i16) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::pitch_bend(channel, value));
        Ok(())
    }

    #[cfg(feature = "midi-io")]
    pub fn send_program_change(&self, channel: u8, program: u8) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::program_change(channel, program));
        Ok(())
    }

    #[cfg(feature = "midi-io")]
    pub fn send_event(&self, event: &MidiEvent) -> Result<()> {
        self.inner
            .output_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI output not enabled".to_string()))?
            .send_message(MidiOutputMessage::from_event(event));
        Ok(())
    }

    /// # Example
    /// ```ignore
    /// midi.send()
    ///     .note_on(0, 60, 100)
    ///     .cc(0, 74, 64)
    ///     .pitch_bend(0, 0);
    /// ```
    pub fn send(&self) -> crate::midi_builder::MidiBuilder<'_> {
        crate::midi_builder::MidiBuilder::new(Some(self))
    }

    pub fn note_on(&self, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_on(0, channel.min(15), note, velocity)
    }

    /// Frame offset enables sample-accurate timing within a buffer.
    pub fn note_on_at(
        &self,
        frame_offset: usize,
        channel: u8,
        note: u8,
        velocity: u8,
    ) -> MidiEvent {
        MidiEvent::note_on(frame_offset, channel.min(15), note, velocity)
    }

    pub fn note_off(&self, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_off(0, channel.min(15), note, velocity)
    }

    pub fn note_off_at(
        &self,
        frame_offset: usize,
        channel: u8,
        note: u8,
        velocity: u8,
    ) -> MidiEvent {
        MidiEvent::note_off(frame_offset, channel.min(15), note, velocity)
    }

    pub fn cc(&self, channel: u8, cc: u8, value: u8) -> MidiEvent {
        MidiEvent::control_change(0, channel.min(15), cc, value)
    }

    pub fn cc_at(&self, frame_offset: usize, channel: u8, cc: u8, value: u8) -> MidiEvent {
        MidiEvent::control_change(frame_offset, channel.min(15), cc, value)
    }

    /// `value`: 14-bit (0-16383, 8192 = center).
    pub fn pitch_bend(&self, channel: u8, value: u16) -> MidiEvent {
        MidiEvent::pitch_bend(0, channel.min(15), value.min(16383))
    }

    pub fn pitch_bend_at(&self, frame_offset: usize, channel: u8, value: u16) -> MidiEvent {
        MidiEvent::pitch_bend(frame_offset, channel.min(15), value.min(16383))
    }

    pub fn channel_pressure(&self, channel: u8, pressure: u8) -> MidiEvent {
        MidiEvent::aftertouch(0, channel.min(15), pressure)
    }

    pub fn poly_pressure(&self, channel: u8, note: u8, pressure: u8) -> MidiEvent {
        MidiEvent::poly_aftertouch(0, channel.min(15), note, pressure)
    }

    #[cfg(feature = "mpe")]
    pub fn mpe(&self) -> MpeHandle {
        MpeHandle::new(self.inner.mpe_processor.clone())
    }

    /// Shared expression state for synth voice rendering.
    #[cfg(feature = "mpe")]
    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.inner
            .mpe_processor
            .as_ref()
            .map(|p| p.read().expression())
    }

    #[cfg(feature = "mpe")]
    pub fn is_mpe_enabled(&self) -> bool {
        self.inner
            .mpe_processor
            .as_ref()
            .map(|p| !matches!(p.read().mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    #[cfg(feature = "midi2")]
    pub fn midi2(&self) -> Midi2Handle {
        Midi2Handle
    }

    /// Inject a MIDI 2.0 event into the processing pipeline without hardware.
    ///
    /// Converts to MIDI 1.0 and pushes through the standard audio pipeline.
    /// MIDI 2.0-only messages (e.g. `PerNotePitchBend`) that have no MIDI 1.0
    /// equivalent are routed to the MPE processor (when `mpe` feature is enabled)
    /// instead.
    #[cfg(feature = "midi2")]
    pub fn push_midi2_event(&self, port_index: usize, event: Midi2Event) -> bool {
        // Also feed MPE processor for MIDI 2.0-only per-note messages
        #[cfg(feature = "mpe")]
        if let Some(ref processor) = self.inner.mpe_processor {
            let unified = crate::event::UnifiedMidiEvent::V2(event);
            processor.write().process_unified(&unified);
        }

        // Convert to MIDI 1.0 and push through standard pipeline
        if let Some(midi1) = event.to_midi1() {
            self.inner.port_manager.push_input_event(port_index, midi1)
        } else {
            // MIDI 2.0-only messages (PerNotePitchBend, etc.) were handled
            // by MPE processor above; no MIDI 1.0 equivalent to push.
            true
        }
    }

    /// Inject a unified MIDI event (1.0 or 2.0) into the processing pipeline.
    #[cfg(feature = "midi2")]
    pub fn push_unified_event(
        &self,
        port_index: usize,
        event: crate::event::UnifiedMidiEvent,
    ) -> bool {
        match event {
            crate::event::UnifiedMidiEvent::V1(midi1) => {
                self.inner.port_manager.push_input_event(port_index, midi1)
            }
            crate::event::UnifiedMidiEvent::V2(midi2) => self.push_midi2_event(port_index, midi2),
        }
    }

    /// Set a UI observer that receives a copy of all incoming MIDI events.
    /// Used by framework integrations (e.g. Bevy) to expose MIDI input as events.
    #[cfg(feature = "midi-io")]
    pub fn set_ui_observer(&self, sender: crossbeam_channel::Sender<MidiEvent>) {
        if let Some(ref manager) = self.inner.input_manager {
            manager.set_ui_observer(sender);
        }
    }

    /// For framework integration; prefer the high-level API methods.
    pub fn port_manager(&self) -> Arc<MidiPortManager> {
        self.inner.port_manager.clone()
    }

    #[cfg(feature = "midi-io")]
    pub fn output_manager(&self) -> Option<Arc<MidiOutputManager>> {
        self.inner.output_manager.clone()
    }

    pub fn cc_manager(&self) -> Option<Arc<crate::cc::CCMappingManager>> {
        self.inner.cc_manager.clone()
    }

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
