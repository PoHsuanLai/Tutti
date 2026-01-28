//! Unified MIDI system with port management, I/O, MPE, and MIDI 2.0.
//!
//! ## Quick Start
//!
//! ```ignore
//! use tutti_midi::{MidiSystem, MpeMode, MpeZoneConfig};
//!
//! // Create MIDI system with I/O and MPE
//! let midi = MidiSystem::new()
//!     .with_io()
//!     .with_mpe(MpeMode::LowerZone(MpeZoneConfig::lower(15)))
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

use crate::event::MidiEvent;
use crate::multi_port::{MidiPortManager, PortInfo};
use crate::error::Result;
use std::sync::Arc;

#[cfg(feature = "midi-io")]
use crate::error::Error;
#[cfg(feature = "midi-io")]
use crate::input::{MidiInputDevice, MidiInputManager};
#[cfg(feature = "midi-io")]
use crate::output::{MidiOutputManager, MidiOutputMessage};

#[cfg(feature = "mpe")]
use crate::mpe::{MpeMode, MpeProcessor, PerNoteExpression};

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

struct MidiSystemInner {
    port_manager: Arc<MidiPortManager>,
    #[cfg(feature = "midi-io")]
    input_manager: Option<Arc<MidiInputManager>>,
    #[cfg(feature = "midi-io")]
    output_manager: Option<Arc<MidiOutputManager>>,
    #[cfg(feature = "mpe")]
    mpe_processor: Option<Arc<MpeProcessor>>,
    cc_manager: Option<Arc<crate::cc_manager::CCMappingManager>>,
    output_collector: Option<Arc<crate::output_collector::MidiOutputAggregator>>,
}

impl MidiSystem {
    /// Create a new MIDI system builder
    ///
    /// # Example
    ///
    /// ```ignore
    /// let midi = MidiSystem::new()
    ///     .with_io()
    ///     .build()?;
    /// ```
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> MidiSystemBuilder {
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
            .map_err(Error::InvalidConfig)
    }

    /// Connect a hardware MIDI device by name (partial match)
    #[cfg(feature = "midi-io")]
    pub fn connect_device_by_name(&self, name: &str) -> Result<()> {
        self.inner
            .input_manager
            .as_ref()
            .ok_or_else(|| Error::InvalidConfig("MIDI I/O not enabled".to_string()))?
            .connect_by_name(name)
            .map_err(Error::InvalidConfig)
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
    pub fn note_on_at(&self, frame_offset: usize, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_on(frame_offset, channel.min(15), note, velocity)
    }

    /// Create a Note Off event
    pub fn note_off(&self, channel: u8, note: u8, velocity: u8) -> MidiEvent {
        MidiEvent::note_off(0, channel.min(15), note, velocity)
    }

    /// Create a Note Off event with frame offset
    pub fn note_off_at(&self, frame_offset: usize, channel: u8, note: u8, velocity: u8) -> MidiEvent {
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
        MpeHandle {
            processor: self.inner.mpe_processor.clone(),
        }
    }

    /// Get the shared per-note expression state (for synth voices)
    #[cfg(feature = "mpe")]
    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.inner.mpe_processor.as_ref().map(|p| p.expression())
    }

    /// Check if MPE is enabled
    #[cfg(feature = "mpe")]
    pub fn is_mpe_enabled(&self) -> bool {
        self.inner
            .mpe_processor
            .as_ref()
            .map(|p| !matches!(p.mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    // ==================== MIDI 2.0 ====================

    /// Get the MIDI 2.0 sub-handle for high-resolution messages
    #[cfg(feature = "midi2")]
    pub fn midi2(&self) -> Midi2Handle {
        Midi2Handle
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
    pub fn cc_manager(&self) -> Option<Arc<crate::cc_manager::CCMappingManager>> {
        self.inner.cc_manager.clone()
    }

    /// Get direct access to the MIDI output collector (advanced usage)
    pub fn output_collector(&self) -> Option<Arc<crate::output_collector::MidiOutputAggregator>> {
        self.inner.output_collector.clone()
    }
}

// ============================================================================
// MidiSystemBuilder
// ============================================================================

/// Builder for configuring MidiSystem
pub struct MidiSystemBuilder {
    #[cfg(feature = "midi-io")]
    enable_io: bool,
    #[cfg(feature = "mpe")]
    mpe_mode: Option<MpeMode>,
    enable_cc_mapping: bool,
    enable_output_collector: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for MidiSystemBuilder {
    fn default() -> Self {
        Self {
            #[cfg(feature = "midi-io")]
            enable_io: false,
            #[cfg(feature = "mpe")]
            mpe_mode: None,
            enable_cc_mapping: false,
            enable_output_collector: false,
        }
    }
}

impl MidiSystemBuilder {
    /// Enable hardware MIDI I/O
    #[cfg(feature = "midi-io")]
    pub fn with_io(mut self) -> Self {
        self.enable_io = true;
        self
    }

    /// Enable MPE with the given mode
    #[cfg(feature = "mpe")]
    pub fn with_mpe(mut self, mode: MpeMode) -> Self {
        self.mpe_mode = Some(mode);
        self
    }

    /// Enable CC mapping manager
    pub fn with_cc_mapping(mut self) -> Self {
        self.enable_cc_mapping = true;
        self
    }

    /// Enable MIDI output collector (for collecting MIDI from audio nodes)
    pub fn with_output_collector(mut self) -> Self {
        self.enable_output_collector = true;
        self
    }

    /// Build the MIDI system
    pub fn build(self) -> Result<MidiSystem> {
        let port_manager = Arc::new(MidiPortManager::new(256));

        #[cfg(feature = "midi-io")]
        let (input_manager, output_manager) = if self.enable_io {
            let input = MidiInputManager::new(port_manager.clone());
            let output = MidiOutputManager::new();
            (Some(Arc::new(input)), Some(Arc::new(output)))
        } else {
            (None, None)
        };

        #[cfg(feature = "mpe")]
        let mpe_processor = self.mpe_mode.map(|mode| Arc::new(MpeProcessor::new(mode)));

        let cc_manager = if self.enable_cc_mapping {
            Some(Arc::new(crate::cc_manager::CCMappingManager::new()))
        } else {
            None
        };

        let output_collector = if self.enable_output_collector {
            Some(Arc::new(crate::output_collector::MidiOutputAggregator::new()))
        } else {
            None
        };

        Ok(MidiSystem {
            inner: Arc::new(MidiSystemInner {
                port_manager,
                #[cfg(feature = "midi-io")]
                input_manager,
                #[cfg(feature = "midi-io")]
                output_manager,
                #[cfg(feature = "mpe")]
                mpe_processor,
                cc_manager,
                output_collector,
            }),
        })
    }
}

// ============================================================================
// Sub-Handles
// ============================================================================

/// Handle for MPE functionality
#[cfg(feature = "mpe")]
pub struct MpeHandle {
    processor: Option<Arc<MpeProcessor>>,
}

#[cfg(feature = "mpe")]
impl MpeHandle {
    /// Get the shared per-note expression state
    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.processor.as_ref().map(|p| p.expression())
    }

    /// Get pitch bend for a note (combined per-note + global)
    ///
    /// Returns normalized value: -1.0 (max down) to 1.0 (max up)
    #[inline]
    pub fn pitch_bend(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_pitch_bend(note))
            .unwrap_or(0.0)
    }

    /// Get per-note pitch bend only (without global)
    #[inline]
    pub fn pitch_bend_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_pitch_bend_per_note(note))
            .unwrap_or(0.0)
    }

    /// Get global pitch bend (from master channel)
    #[inline]
    pub fn pitch_bend_global(&self) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_pitch_bend_global())
            .unwrap_or(0.0)
    }

    /// Get pressure for a note (max of per-note and global)
    ///
    /// Returns normalized value: 0.0 to 1.0
    #[inline]
    pub fn pressure(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_pressure(note))
            .unwrap_or(0.0)
    }

    /// Get per-note pressure only
    #[inline]
    pub fn pressure_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_pressure_per_note(note))
            .unwrap_or(0.0)
    }

    /// Get slide (CC74) for a note
    ///
    /// Returns normalized value: 0.0 to 1.0
    #[inline]
    pub fn slide(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.expression().get_slide(note))
            .unwrap_or(0.5)
    }

    /// Check if a note is currently active
    #[inline]
    pub fn is_note_active(&self, note: u8) -> bool {
        self.processor
            .as_ref()
            .map(|p| p.expression().is_active(note))
            .unwrap_or(false)
    }

    /// Get the current MPE mode
    pub fn mode(&self) -> Option<&MpeMode> {
        self.processor.as_ref().map(|p| p.mode())
    }

    /// Check if MPE is enabled
    pub fn is_enabled(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| !matches!(p.mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    /// Check if using lower zone
    pub fn has_lower_zone(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| {
                matches!(
                    p.mode(),
                    MpeMode::LowerZone(_) | MpeMode::DualZone { .. }
                )
            })
            .unwrap_or(false)
    }

    /// Check if using upper zone
    pub fn has_upper_zone(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| {
                matches!(
                    p.mode(),
                    MpeMode::UpperZone(_) | MpeMode::DualZone { .. }
                )
            })
            .unwrap_or(false)
    }
}

/// Handle for MIDI 2.0 functionality
#[cfg(feature = "midi2")]
pub struct Midi2Handle;

#[cfg(feature = "midi2")]
impl Midi2Handle {
    // ==================== Event Creation ====================

    /// Create a MIDI 2.0 Note On event
    ///
    /// * `note` - MIDI note number (0-127)
    /// * `velocity` - Normalized velocity (0.0-1.0)
    /// * `channel` - MIDI channel (0-15)
    pub fn note_on(&self, note: u8, velocity: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let vel16 = (velocity.clamp(0.0, 1.0) * 65535.0) as u16;
        Midi2Event::note_on(
            0,
            u4::new(0),
            u4::new(channel.min(15)),
            u7::new(note.min(127)),
            vel16,
        )
    }

    /// Create a MIDI 2.0 Note Off event
    pub fn note_off(&self, note: u8, velocity: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let vel16 = (velocity.clamp(0.0, 1.0) * 65535.0) as u16;
        Midi2Event::note_off(
            0,
            u4::new(0),
            u4::new(channel.min(15)),
            u7::new(note.min(127)),
            vel16,
        )
    }

    /// Create a MIDI 2.0 per-note pitch bend event
    ///
    /// * `note` - MIDI note number (0-127)
    /// * `bend` - Normalized pitch bend (-1.0 to 1.0, 0.0 = center)
    /// * `channel` - MIDI channel (0-15)
    pub fn per_note_pitch_bend(&self, note: u8, bend: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let bend_clamped = bend.clamp(-1.0, 1.0);
        let bend32 = ((bend_clamped as f64 + 1.0) * 0x80000000_u32 as f64) as u32;
        Midi2Event::per_note_pitch_bend(
            0,
            u4::new(0),
            u4::new(channel.min(15)),
            u7::new(note.min(127)),
            bend32,
        )
    }

    /// Create a MIDI 2.0 channel pitch bend event
    pub fn channel_pitch_bend(&self, bend: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let bend_clamped = bend.clamp(-1.0, 1.0);
        let bend32 = ((bend_clamped as f64 + 1.0) * 0x80000000_u32 as f64) as u32;
        Midi2Event::channel_pitch_bend(0, u4::new(0), u4::new(channel.min(15)), bend32)
    }

    /// Create a MIDI 2.0 Control Change event
    pub fn control_change(&self, cc: u8, value: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let val32 = (value.clamp(0.0, 1.0) as f64 * 0xFFFFFFFF_u32 as f64) as u32;
        Midi2Event::control_change(
            0,
            u4::new(0),
            u4::new(channel.min(15)),
            u7::new(cc.min(127)),
            val32,
        )
    }

    /// Create a MIDI 2.0 per-note pressure (poly aftertouch) event
    pub fn key_pressure(&self, note: u8, pressure: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let press32 = (pressure.clamp(0.0, 1.0) as f64 * 0xFFFFFFFF_u32 as f64) as u32;
        Midi2Event::key_pressure(
            0,
            u4::new(0),
            u4::new(channel.min(15)),
            u7::new(note.min(127)),
            press32,
        )
    }

    /// Create a MIDI 2.0 channel pressure (aftertouch) event
    pub fn channel_pressure(&self, pressure: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let press32 = (pressure.clamp(0.0, 1.0) as f64 * 0xFFFFFFFF_u32 as f64) as u32;
        Midi2Event::channel_pressure(0, u4::new(0), u4::new(channel.min(15)), press32)
    }

    // ==================== Conversion ====================

    /// Convert a MIDI 1.0 event to MIDI 2.0 (upsamples resolution)
    pub fn convert_to_midi2(&self, event: &MidiEvent) -> Option<Midi2Event> {
        crate::midi2::midi1_to_midi2(event)
    }

    /// Convert a MIDI 2.0 event to MIDI 1.0 (downsamples resolution)
    pub fn convert_to_midi1(&self, event: &Midi2Event) -> Option<MidiEvent> {
        event.to_midi1()
    }

    // ==================== Value Conversion Utilities ====================

    /// Convert 7-bit MIDI 1.0 velocity to 16-bit MIDI 2.0
    #[inline]
    pub fn velocity_to_16bit(&self, v: u8) -> u16 {
        crate::midi2::midi1_velocity_to_midi2(v)
    }

    /// Convert 16-bit MIDI 2.0 velocity to 7-bit MIDI 1.0
    #[inline]
    pub fn velocity_to_7bit(&self, v: u16) -> u8 {
        crate::midi2::midi2_velocity_to_midi1(v)
    }

    /// Convert 7-bit MIDI 1.0 CC value to 32-bit MIDI 2.0
    #[inline]
    pub fn cc_to_32bit(&self, v: u8) -> u32 {
        crate::midi2::midi1_cc_to_midi2(v)
    }

    /// Convert 32-bit MIDI 2.0 CC value to 7-bit MIDI 1.0
    #[inline]
    pub fn cc_to_7bit(&self, v: u32) -> u8 {
        crate::midi2::midi2_cc_to_midi1(v)
    }

    /// Convert 14-bit MIDI 1.0 pitch bend to 32-bit MIDI 2.0
    #[inline]
    pub fn pitch_bend_to_32bit(&self, v: u16) -> u32 {
        crate::midi2::midi1_pitch_bend_to_midi2(v)
    }

    /// Convert 32-bit MIDI 2.0 pitch bend to 14-bit MIDI 1.0
    #[inline]
    pub fn pitch_bend_to_14bit(&self, v: u32) -> u16 {
        crate::midi2::midi2_pitch_bend_to_midi1(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_midi_system_creation() {
        let midi = MidiSystem::new().build().unwrap();

        let port_id = midi.create_input_port("Test");
        assert_eq!(port_id, 0);

        let ports = midi.list_input_ports();
        assert_eq!(ports.len(), 1);
        assert_eq!(ports[0].name, "Test");
    }

    #[test]
    fn test_event_creation() {
        let midi = MidiSystem::new().build().unwrap();

        let event = midi.note_on(0, 60, 100);
        assert!(event.is_note_on());
        assert_eq!(event.note(), Some(60));
    }

    #[test]
    fn test_clone() {
        let midi = MidiSystem::new().build().unwrap();
        let midi2 = midi.clone();

        midi.create_input_port("Port1");
        let ports = midi2.list_input_ports();
        assert_eq!(ports.len(), 1);
    }

    #[cfg(feature = "mpe")]
    #[test]
    fn test_mpe_builder() {
        use crate::mpe::MpeZoneConfig;

        let midi = MidiSystem::new()
            .with_mpe(MpeMode::LowerZone(MpeZoneConfig::lower(15)))
            .build()
            .unwrap();

        assert!(midi.is_mpe_enabled());
        assert!(midi.mpe().has_lower_zone());
    }
}
