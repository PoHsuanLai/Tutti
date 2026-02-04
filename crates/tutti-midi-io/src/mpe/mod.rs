//! MPE (MIDI Polyphonic Expression) per-note pitch bend, pressure, and slide.

// Many public methods are only called from `midi-io` or `midi2` feature gates,
// but they are part of the public API and tested directly.
#![allow(dead_code)]

use std::sync::Arc;

use crate::MidiEvent;
use midi_msg::ChannelVoiceMsg;

mod expression;
mod voice_map;
mod zone;

pub use expression::PerNoteExpression;
pub use zone::{MpeMode, MpeZone, MpeZoneConfig};

pub(crate) use voice_map::{MpeChannelVoiceMap, ZoneInfo};

/// Processes MIDI input and routes to per-note expression
///
/// The processor handles:
/// - MIDI 1.0 MPE (channel-based) via zone configuration
/// - MIDI 2.0 per-note messages (when `midi2` feature is enabled)
///
/// ## Usage
///
/// ```ignore
/// let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));
///
/// // In audio callback:
/// for event in midi_events {
///     processor.process_midi1(&event);
/// }
///
/// // In synth voice:
/// let pitch_bend = processor.expression().get_pitch_bend(note);
/// ```
pub struct MpeProcessor {
    /// MPE mode configuration
    mode: MpeMode,
    /// Per-note expression state (shared with synth voices)
    expression: Arc<PerNoteExpression>,
    /// Channel-voice mapping for lower zone
    lower_zone_map: Option<MpeChannelVoiceMap>,
    /// Channel-voice mapping for upper zone
    upper_zone_map: Option<MpeChannelVoiceMap>,
}

impl MpeProcessor {
    /// Create a new MPE processor
    pub fn new(mode: MpeMode) -> Self {
        let expression = Arc::new(PerNoteExpression::new());

        let (lower_zone_map, upper_zone_map) = match &mode {
            MpeMode::Disabled => (None, None),
            MpeMode::LowerZone(config) => (Some(MpeChannelVoiceMap::new(config.clone())), None),
            MpeMode::UpperZone(config) => (None, Some(MpeChannelVoiceMap::new(config.clone()))),
            MpeMode::DualZone { lower, upper } => (
                Some(MpeChannelVoiceMap::new(lower.clone())),
                Some(MpeChannelVoiceMap::new(upper.clone())),
            ),
        };

        Self {
            mode,
            expression,
            lower_zone_map,
            upper_zone_map,
        }
    }

    /// Get the shared expression state
    ///
    /// Use this to read expression values from synth voices.
    pub fn expression(&self) -> Arc<PerNoteExpression> {
        Arc::clone(&self.expression)
    }

    /// Get the current MPE mode
    pub fn mode(&self) -> &MpeMode {
        &self.mode
    }

    /// Process a MIDI 1.0 event
    ///
    /// Routes pitch bend, pressure, and CC74 to per-note expression
    /// based on the channel-to-note mapping.
    pub fn process_midi1(&mut self, event: &MidiEvent) {
        if matches!(self.mode, MpeMode::Disabled) {
            return;
        }

        let channel = event.channel_num();

        // Check which zone handles this channel
        let zone_info = self.get_zone_info(channel);
        let zone_info = match zone_info {
            Some(info) => info,
            None => return, // Channel not in any zone
        };

        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                if velocity > 0 {
                    // Assign channel to note
                    if zone_info.is_member {
                        // For member channels, register this channel->note mapping
                        if let Some(ref mut map) = self.get_voice_map_mut(zone_info.is_lower_zone) {
                            map.channel_to_note[channel as usize] = Some(note);
                            map.note_to_channel[note as usize] = Some(channel);
                        }
                    }
                    self.expression.note_on(note);
                } else {
                    // Velocity 0 = Note Off
                    self.handle_note_off_internal(channel, note, zone_info.is_lower_zone);
                }
            }
            ChannelVoiceMsg::NoteOff { note, .. } => {
                self.handle_note_off_internal(channel, note, zone_info.is_lower_zone);
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                // Convert 14-bit (0-16383, center 8192) to -1.0..1.0
                let normalized = (bend as f32 - 8192.0) / 8192.0;

                if zone_info.is_master {
                    // Master channel affects all notes globally
                    self.expression.set_global_pitch_bend(normalized);
                } else if zone_info.is_member {
                    // Member channel affects specific note
                    if let Some(ref map) = self.get_voice_map(zone_info.is_lower_zone) {
                        if let Some(note) = map.get_note_for_channel(channel) {
                            self.expression.set_pitch_bend(note, normalized);
                        }
                    }
                }
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                // Convert 7-bit (0-127) to 0.0..1.0
                let normalized = pressure as f32 / 127.0;

                if zone_info.is_master {
                    self.expression.set_global_pressure(normalized);
                } else if zone_info.is_member {
                    if let Some(ref map) = self.get_voice_map(zone_info.is_lower_zone) {
                        if let Some(note) = map.get_note_for_channel(channel) {
                            self.expression.set_pressure(note, normalized);
                        }
                    }
                }
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                // Poly pressure directly specifies the note
                let normalized = pressure as f32 / 127.0;
                self.expression.set_pressure(note, normalized);
            }
            ChannelVoiceMsg::ControlChange {
                control: midi_msg::ControlChange::CC { control: cc, value },
            } => {
                if cc == 74 {
                    // CC74 = Slide (MPE standard)
                    let normalized = value as f32 / 127.0;

                    if zone_info.is_member {
                        if let Some(ref map) = self.get_voice_map(zone_info.is_lower_zone) {
                            if let Some(note) = map.get_note_for_channel(channel) {
                                self.expression.set_slide(note, normalized);
                            }
                        }
                    }
                }
                // Other CCs could be handled here (e.g., CC1 for modulation)
            }
            _ => {}
        }
    }

    /// Handle note off - release channel mapping
    fn handle_note_off_internal(&mut self, channel: u8, note: u8, is_lower_zone: bool) {
        if let Some(ref mut map) = self.get_voice_map_mut(is_lower_zone) {
            // For member channels, clear the mapping
            if map.handles_channel(channel) {
                map.channel_to_note[channel as usize] = None;
                map.note_to_channel[note as usize] = None;
            }
        }
        self.expression.note_off(note);
    }

    /// Zone info for a channel
    fn get_zone_info(&self, channel: u8) -> Option<ZoneInfo> {
        match &self.mode {
            MpeMode::Disabled => None,
            MpeMode::LowerZone(config) => {
                if config.handles_channel(channel) {
                    Some(ZoneInfo {
                        is_master: config.is_master_channel(channel),
                        is_member: config.is_member_channel(channel),
                        is_lower_zone: true,
                    })
                } else {
                    None
                }
            }
            MpeMode::UpperZone(config) => {
                if config.handles_channel(channel) {
                    Some(ZoneInfo {
                        is_master: config.is_master_channel(channel),
                        is_member: config.is_member_channel(channel),
                        is_lower_zone: false,
                    })
                } else {
                    None
                }
            }
            MpeMode::DualZone { lower, upper } => {
                if lower.handles_channel(channel) {
                    Some(ZoneInfo {
                        is_master: lower.is_master_channel(channel),
                        is_member: lower.is_member_channel(channel),
                        is_lower_zone: true,
                    })
                } else if upper.handles_channel(channel) {
                    Some(ZoneInfo {
                        is_master: upper.is_master_channel(channel),
                        is_member: upper.is_member_channel(channel),
                        is_lower_zone: false,
                    })
                } else {
                    None
                }
            }
        }
    }

    /// Get voice map reference
    fn get_voice_map(&self, is_lower_zone: bool) -> &Option<MpeChannelVoiceMap> {
        if is_lower_zone {
            &self.lower_zone_map
        } else {
            &self.upper_zone_map
        }
    }

    /// Get voice map mutable reference
    fn get_voice_map_mut(&mut self, is_lower_zone: bool) -> &mut Option<MpeChannelVoiceMap> {
        if is_lower_zone {
            &mut self.lower_zone_map
        } else {
            &mut self.upper_zone_map
        }
    }

    /// Process a MIDI 2.0 event (per-note messages)
    ///
    /// MIDI 2.0 has native per-note pitch bend and controllers,
    /// so no channel-voice mapping is needed.
    #[cfg(feature = "midi2")]
    pub fn process_midi2(&self, event: &crate::midi2::Midi2Event) {
        use crate::midi2::Midi2MessageType;

        match event.message_type() {
            Midi2MessageType::NoteOn { note, .. } => {
                self.expression.note_on(note);
            }
            Midi2MessageType::NoteOff { note, .. } => {
                self.expression.note_off(note);
            }
            Midi2MessageType::PerNotePitchBend { note, bend } => {
                // MIDI 2.0 pitch bend: 32-bit, center at 0x80000000
                let normalized = (bend as f64 - 0x80000000_u32 as f64) / 0x80000000_u32 as f64;
                self.expression.set_pitch_bend(note, normalized as f32);
            }
            Midi2MessageType::KeyPressure { note, pressure } => {
                // MIDI 2.0 pressure: 32-bit, 0 to 0xFFFFFFFF
                let normalized = pressure as f64 / 0xFFFFFFFF_u32 as f64;
                self.expression.set_pressure(note, normalized as f32);
            }
            Midi2MessageType::ChannelPitchBend { bend } => {
                let normalized = (bend as f64 - 0x80000000_u32 as f64) / 0x80000000_u32 as f64;
                self.expression.set_global_pitch_bend(normalized as f32);
            }
            Midi2MessageType::ChannelPressure { pressure } => {
                let normalized = pressure as f64 / 0xFFFFFFFF_u32 as f64;
                self.expression.set_global_pressure(normalized as f32);
            }
            Midi2MessageType::AssignablePerNoteController { note, index, value } => {
                // CC74 (slide) is controller index 74
                if index == 74 {
                    let normalized = value as f64 / 0xFFFFFFFF_u32 as f64;
                    self.expression.set_slide(note, normalized as f32);
                }
            }
            _ => {}
        }
    }

    /// Process a unified MIDI event (dispatches to MIDI 1.0 or 2.0 handler)
    #[cfg(feature = "midi2")]
    pub fn process_unified(&mut self, event: &crate::event::UnifiedMidiEvent) {
        match event {
            crate::event::UnifiedMidiEvent::V1(e) => self.process_midi1(e),
            crate::event::UnifiedMidiEvent::V2(e) => self.process_midi2(e),
        }
    }

    // ========================================================================
    // Outgoing MPE: Send notes with automatic channel allocation
    // ========================================================================

    /// Allocate a channel for a note and return the channel to send on
    ///
    /// This is for **outgoing** MPE: converting a note to an MPE channel assignment.
    /// Returns the allocated channel, or None if MPE is disabled.
    ///
    /// # Example
    /// ```ignore
    /// let channel = processor.allocate_channel_for_note(60)?;
    /// midi_out.send_note_on(channel, 60, 100)?;
    /// ```
    pub fn allocate_channel_for_note(&mut self, note: u8) -> Option<u8> {
        match &self.mode {
            MpeMode::Disabled => None,
            MpeMode::LowerZone(_) => {
                if let Some(ref mut map) = self.lower_zone_map {
                    map.assign_note(note)
                } else {
                    None
                }
            }
            MpeMode::UpperZone(_) => {
                if let Some(ref mut map) = self.upper_zone_map {
                    map.assign_note(note)
                } else {
                    None
                }
            }
            MpeMode::DualZone { .. } => {
                // For dual zone, prefer lower zone
                if let Some(ref mut map) = self.lower_zone_map {
                    map.assign_note(note)
                } else if let Some(ref mut map) = self.upper_zone_map {
                    map.assign_note(note)
                } else {
                    None
                }
            }
        }
    }

    /// Release a note's channel allocation
    ///
    /// Call this on Note Off to free up the channel for reuse.
    pub fn release_channel_for_note(&mut self, note: u8) {
        if let Some(ref mut map) = self.lower_zone_map {
            map.release_note(note);
        }
        if let Some(ref mut map) = self.upper_zone_map {
            map.release_note(note);
        }
    }

    /// Get the channel currently assigned to a note
    ///
    /// Returns None if the note is not currently playing or MPE is disabled.
    pub fn get_channel_for_note(&self, note: u8) -> Option<u8> {
        // Check lower zone first
        if let Some(ref map) = self.lower_zone_map {
            if let Some(ch) = map.get_channel_for_note(note) {
                return Some(ch);
            }
        }
        // Then check upper zone
        if let Some(ref map) = self.upper_zone_map {
            if let Some(ch) = map.get_channel_for_note(note) {
                return Some(ch);
            }
        }
        None
    }

    /// Reset all expression state
    pub fn reset(&mut self) {
        self.expression.reset();
        if let Some(ref mut map) = self.lower_zone_map {
            map.clear();
        }
        if let Some(ref mut map) = self.upper_zone_map {
            map.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_per_note_expression() {
        let expr = PerNoteExpression::new();

        expr.note_on(60);
        assert!(expr.is_active(60));

        expr.set_pitch_bend(60, 0.5);
        assert!((expr.get_pitch_bend(60) - 0.5).abs() < 0.001);

        expr.set_pressure(60, 0.75);
        assert!((expr.get_pressure(60) - 0.75).abs() < 0.001);

        expr.set_slide(60, 0.3);
        assert!((expr.get_slide(60) - 0.3).abs() < 0.001);

        expr.note_off(60);
        assert!(!expr.is_active(60));
    }

    #[test]
    fn test_global_expression() {
        let expr = PerNoteExpression::new();

        expr.note_on(60);
        expr.set_pitch_bend(60, 0.2);
        expr.set_global_pitch_bend(0.3);

        // Combined pitch bend should be 0.5
        assert!((expr.get_pitch_bend(60) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_expression_clamping() {
        let expr = PerNoteExpression::new();

        expr.set_pitch_bend(60, 2.0);
        assert!((expr.get_pitch_bend(60) - 1.0).abs() < 0.001);

        expr.set_pitch_bend(60, -2.0);
        assert!((expr.get_pitch_bend(60) - (-1.0)).abs() < 0.001);

        expr.set_pressure(60, 1.5);
        assert!((expr.get_pressure(60) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_mpe_processor_pitch_bend() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        // Note on channel 2 (member channel)
        let note_on = MidiEvent::note_on(0, 2, 60, 100);
        processor.process_midi1(&note_on);

        // Pitch bend on channel 2
        let pitch_bend = MidiEvent::pitch_bend(0, 2, 16383); // Max up
        processor.process_midi1(&pitch_bend);

        // Should have max pitch bend
        let bend = processor.expression().get_pitch_bend(60);
        assert!((bend - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_mpe_processor_master_channel() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        // Note on channel 2 (member channel)
        let note_on = MidiEvent::note_on(0, 2, 60, 100);
        processor.process_midi1(&note_on);

        // Global pitch bend on master channel (0)
        let pitch_bend = MidiEvent::pitch_bend(0, 0, 12288); // ~0.5 up
        processor.process_midi1(&pitch_bend);

        // Should have global pitch bend
        let global = processor.expression().get_pitch_bend_global();
        assert!(global > 0.4 && global < 0.6);
    }

    #[test]
    fn test_mpe_disabled() {
        let mut processor = MpeProcessor::new(MpeMode::Disabled);

        // Events should be ignored when disabled
        let note_on = MidiEvent::note_on(0, 2, 60, 100);
        processor.process_midi1(&note_on);

        assert!(!processor.expression().is_active(60));
    }

    #[cfg(feature = "midi2")]
    #[test]
    fn test_midi2_per_note_pitch_bend() {
        use midi2::prelude::*;

        let processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        // Note on
        let note_on =
            crate::midi2::Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 65535);
        processor.process_midi2(&note_on);

        // Per-note pitch bend (max up)
        let pitch_bend = crate::midi2::Midi2Event::per_note_pitch_bend(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            0xFFFFFFFF,
        );
        processor.process_midi2(&pitch_bend);

        let bend = processor.expression().get_pitch_bend(60);
        assert!((bend - 1.0).abs() < 0.01);
    }
}
