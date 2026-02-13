//! MPE (MIDI Polyphonic Expression) per-note pitch bend, pressure, and slide.

// Many public methods are only called from `midi-io` or `midi2` feature gates,
// but they are part of the public API and tested directly.
#![allow(dead_code)]

use std::sync::Arc;

use crate::ChannelVoiceMsg;
use crate::MidiEvent;

mod expression;
mod voice_map;
mod zone;

pub use expression::PerNoteExpression;
pub use zone::{MpeMode, MpeZone, MpeZoneConfig};

pub(crate) use voice_map::{MpeChannelVoiceMap, ZoneInfo};

/// Routes MIDI input (1.0 channel-based or 2.0 per-note) to per-note expression state.
pub struct MpeProcessor {
    mode: MpeMode,
    /// Shared with synth voices via `Arc::clone`
    expression: Arc<PerNoteExpression>,
    lower_zone_map: Option<MpeChannelVoiceMap>,
    upper_zone_map: Option<MpeChannelVoiceMap>,
}

impl MpeProcessor {
    pub fn new(mode: MpeMode) -> Self {
        let expression = Arc::new(PerNoteExpression::new());

        let (lower_zone_map, upper_zone_map) = match &mode {
            MpeMode::Disabled => (None, None),
            MpeMode::LowerZone(config) => (Some(MpeChannelVoiceMap::new(*config)), None),
            MpeMode::UpperZone(config) => (None, Some(MpeChannelVoiceMap::new(*config))),
            MpeMode::DualZone { lower, upper } => (
                Some(MpeChannelVoiceMap::new(*lower)),
                Some(MpeChannelVoiceMap::new(*upper)),
            ),
        };

        Self {
            mode,
            expression,
            lower_zone_map,
            upper_zone_map,
        }
    }

    pub fn expression(&self) -> Arc<PerNoteExpression> {
        Arc::clone(&self.expression)
    }

    pub fn mode(&self) -> &MpeMode {
        &self.mode
    }

    /// Routes pitch bend, pressure, and CC74 (slide) to per-note expression
    /// based on the channel-to-note mapping.
    pub fn process_midi1(&mut self, event: &MidiEvent) {
        if matches!(self.mode, MpeMode::Disabled) {
            return;
        }

        let channel = event.channel_num();

        let zone_info = self.get_zone_info(channel);
        let zone_info = match zone_info {
            Some(info) => info,
            None => return,
        };

        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                if velocity > 0 {
                    if zone_info.is_member {
                        if let Some(ref mut map) = self.get_voice_map_mut(zone_info.is_lower_zone) {
                            map.channel_to_note[channel as usize] = Some(note);
                            if note < 128 {
                                map.note_to_channel[note as usize] = Some(channel);
                            }
                        }
                    }
                    self.expression.note_on(note);
                } else {
                    self.handle_note_off_internal(channel, note, zone_info.is_lower_zone);
                }
            }
            ChannelVoiceMsg::NoteOff { note, .. } => {
                self.handle_note_off_internal(channel, note, zone_info.is_lower_zone);
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                // 14-bit (0-16383, center 8192) -> -1.0..1.0
                let normalized = (bend as f32 - 8192.0) / 8192.0;

                if zone_info.is_master {
                    self.expression.set_global_pitch_bend(normalized);
                } else if zone_info.is_member {
                    if let Some(ref map) = self.get_voice_map(zone_info.is_lower_zone) {
                        if let Some(note) = map.get_note_for_channel(channel) {
                            self.expression.set_pitch_bend(note, normalized);
                        }
                    }
                }
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                // 7-bit (0-127) -> 0.0..1.0
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
                let normalized = pressure as f32 / 127.0;
                self.expression.set_pressure(note, normalized);
            }
            ChannelVoiceMsg::ControlChange {
                control: crate::ControlChange::CC { control: cc, value },
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
            }
            _ => {}
        }
    }

    fn handle_note_off_internal(&mut self, channel: u8, note: u8, is_lower_zone: bool) {
        if let Some(ref mut map) = self.get_voice_map_mut(is_lower_zone) {
            if map.handles_channel(channel) {
                map.channel_to_note[channel as usize] = None;
                if note < 128 {
                    map.note_to_channel[note as usize] = None;
                }
            }
        }
        self.expression.note_off(note);
    }

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

    fn get_voice_map(&self, is_lower_zone: bool) -> &Option<MpeChannelVoiceMap> {
        if is_lower_zone {
            &self.lower_zone_map
        } else {
            &self.upper_zone_map
        }
    }

    fn get_voice_map_mut(&mut self, is_lower_zone: bool) -> &mut Option<MpeChannelVoiceMap> {
        if is_lower_zone {
            &mut self.lower_zone_map
        } else {
            &mut self.upper_zone_map
        }
    }

    /// MIDI 2.0 has native per-note messages, so no channel-voice mapping is needed.
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

    #[cfg(feature = "midi2")]
    pub fn process_unified(&mut self, event: &crate::event::UnifiedMidiEvent) {
        match event {
            crate::event::UnifiedMidiEvent::V1(e) => self.process_midi1(e),
            crate::event::UnifiedMidiEvent::V2(e) => self.process_midi2(e),
        }
    }

    /// Allocate an MPE member channel for outgoing note-on.
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

    /// Call on Note Off to free up the channel for reuse.
    pub fn release_channel_for_note(&mut self, note: u8) {
        if let Some(ref mut map) = self.lower_zone_map {
            map.release_note(note);
        }
        if let Some(ref mut map) = self.upper_zone_map {
            map.release_note(note);
        }
    }

    pub fn get_channel_for_note(&self, note: u8) -> Option<u8> {
        if let Some(ref map) = self.lower_zone_map {
            if let Some(ch) = map.get_channel_for_note(note) {
                return Some(ch);
            }
        }
        if let Some(ref map) = self.upper_zone_map {
            if let Some(ch) = map.get_channel_for_note(note) {
                return Some(ch);
            }
        }
        None
    }

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

    // -----------------------------------------------------------------------
    // Upper Zone tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_upper_zone_pitch_bend_routes_correctly() {
        // Upper zone: master=Ch16(15), members=Ch15(14)..Ch11(10) — 5 member channels
        let mut processor = MpeProcessor::new(MpeMode::UpperZone(MpeZoneConfig::upper(5)));

        // Note on channel 14 (member channel in upper zone)
        processor.process_midi1(&MidiEvent::note_on(0, 14, 60, 100));

        // Pitch bend on channel 14 → should affect note 60
        processor.process_midi1(&MidiEvent::pitch_bend(0, 14, 16383));
        let bend = processor.expression().get_pitch_bend(60);
        assert!(
            (bend - 1.0).abs() < 0.01,
            "Upper zone member pitch bend failed"
        );

        // Master channel pitch bend (ch 15) → global
        processor.process_midi1(&MidiEvent::pitch_bend(0, 15, 0)); // Max down
        let global = processor.expression().get_pitch_bend_global();
        assert!(
            (global - (-1.0)).abs() < 0.01,
            "Upper zone master pitch bend failed"
        );
    }

    #[test]
    fn test_upper_zone_rejects_lower_channels() {
        let mut processor = MpeProcessor::new(MpeMode::UpperZone(MpeZoneConfig::upper(5)));

        // Channel 0 is not in upper zone — note should NOT become active
        processor.process_midi1(&MidiEvent::note_on(0, 0, 60, 100));
        assert!(!processor.expression().is_active(60));

        // Channel 9 is below the upper zone range (10-14) — should also be rejected
        processor.process_midi1(&MidiEvent::note_on(0, 9, 72, 100));
        assert!(!processor.expression().is_active(72));
    }

    #[test]
    fn test_upper_zone_channel_pressure() {
        let mut processor = MpeProcessor::new(MpeMode::UpperZone(MpeZoneConfig::upper(5)));

        // Note on channel 12 (member)
        processor.process_midi1(&MidiEvent::note_on(0, 12, 60, 100));

        // Channel pressure on channel 12 → per-note pressure for note 60
        processor.process_midi1(&MidiEvent::aftertouch(0, 12, 127));
        let pressure = processor.expression().get_pressure_per_note(60);
        assert!((pressure - 1.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Dual Zone tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dual_zone_routes_to_correct_zone() {
        // Lower: master=0, 7 members (1-7)
        // Upper: master=15, 7 members (8-14)
        let mut processor = MpeProcessor::new(MpeMode::DualZone {
            lower: MpeZoneConfig::lower(7),
            upper: MpeZoneConfig::upper(7),
        });

        // Note on lower zone (ch 3, member)
        processor.process_midi1(&MidiEvent::note_on(0, 3, 60, 100));
        assert!(processor.expression().is_active(60));

        // Note on upper zone (ch 12, member)
        processor.process_midi1(&MidiEvent::note_on(0, 12, 72, 100));
        assert!(processor.expression().is_active(72));

        // Pitch bend on ch 3 → affects note 60, not 72
        processor.process_midi1(&MidiEvent::pitch_bend(0, 3, 16383));
        let bend_60 = processor.expression().get_pitch_bend_per_note(60);
        let bend_72 = processor.expression().get_pitch_bend_per_note(72);
        assert!(
            (bend_60 - 1.0).abs() < 0.01,
            "Lower zone bend should affect note 60"
        );
        assert!(
            (bend_72 - 0.0).abs() < 0.01,
            "Lower zone bend should NOT affect note 72"
        );

        // Pitch bend on ch 12 → affects note 72, not 60
        processor.process_midi1(&MidiEvent::pitch_bend(0, 12, 0));
        let bend_72 = processor.expression().get_pitch_bend_per_note(72);
        assert!(
            (bend_72 - (-1.0)).abs() < 0.01,
            "Upper zone bend should affect note 72"
        );
        // note 60 per-note should still be 1.0
        let bend_60 = processor.expression().get_pitch_bend_per_note(60);
        assert!((bend_60 - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_dual_zone_master_channels_independent() {
        let mut processor = MpeProcessor::new(MpeMode::DualZone {
            lower: MpeZoneConfig::lower(7),
            upper: MpeZoneConfig::upper(7),
        });

        // Lower master (ch 0) pitch bend
        processor.process_midi1(&MidiEvent::pitch_bend(0, 0, 12288));
        let global = processor.expression().get_pitch_bend_global();
        assert!(global > 0.4, "Lower master pitch bend should set global");

        // Upper master (ch 15) pitch bend — overwrites global
        processor.process_midi1(&MidiEvent::pitch_bend(0, 15, 0));
        let global = processor.expression().get_pitch_bend_global();
        assert!(
            (global - (-1.0)).abs() < 0.01,
            "Upper master pitch bend should overwrite global"
        );
    }

    // -----------------------------------------------------------------------
    // Channel allocation / release (outgoing MPE)
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocate_and_release_channel_roundtrip() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(3)));

        // Allocate channel for note 60
        let ch = processor.allocate_channel_for_note(60);
        assert!(ch.is_some());
        let ch60 = ch.unwrap();
        assert!(ch60 >= 1 && ch60 <= 3, "Should be a member channel (1-3)");

        // Look it up
        assert_eq!(processor.get_channel_for_note(60), Some(ch60));

        // Allocate another note — should get a different channel
        let ch64 = processor.allocate_channel_for_note(64).unwrap();
        assert_ne!(ch60, ch64);
        assert_eq!(processor.get_channel_for_note(64), Some(ch64));

        // Release note 60 — channel should be freed
        processor.release_channel_for_note(60);
        assert!(processor.get_channel_for_note(60).is_none());
        // Note 64 should still be assigned
        assert_eq!(processor.get_channel_for_note(64), Some(ch64));
    }

    #[test]
    fn test_allocate_returns_none_when_disabled() {
        let mut processor = MpeProcessor::new(MpeMode::Disabled);
        assert!(processor.allocate_channel_for_note(60).is_none());
    }

    #[test]
    fn test_allocate_voice_stealing_when_channels_exhausted() {
        // Only 2 member channels (1, 2)
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(2)));

        let ch1 = processor.allocate_channel_for_note(60).unwrap();
        let ch2 = processor.allocate_channel_for_note(64).unwrap();
        assert_ne!(ch1, ch2);

        // Both channels occupied — allocating a 3rd note should steal
        let ch3 = processor.allocate_channel_for_note(67).unwrap();
        assert!(ch3 == ch1 || ch3 == ch2, "Should steal an existing channel");

        // The stolen note should be unmapped
        let stolen_note = if ch3 == ch1 { 60 } else { 64 };
        assert!(
            processor.get_channel_for_note(stolen_note).is_none(),
            "Stolen note should be unmapped"
        );

        // New note should be mapped
        assert_eq!(processor.get_channel_for_note(67), Some(ch3));
    }

    #[test]
    fn test_allocate_same_note_twice_returns_same_channel() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(3)));

        let ch1 = processor.allocate_channel_for_note(60).unwrap();
        let ch2 = processor.allocate_channel_for_note(60).unwrap();
        assert_eq!(
            ch1, ch2,
            "Re-allocating same note should return same channel"
        );
    }

    #[test]
    fn test_allocate_upper_zone() {
        let mut processor = MpeProcessor::new(MpeMode::UpperZone(MpeZoneConfig::upper(3)));

        let ch = processor.allocate_channel_for_note(60).unwrap();
        // Upper zone with 3 members: channels 12, 13, 14
        assert!(
            ch >= 12 && ch <= 14,
            "Upper zone channel should be 12-14, got {ch}"
        );
    }

    #[test]
    fn test_allocate_dual_zone_prefers_lower() {
        let mut processor = MpeProcessor::new(MpeMode::DualZone {
            lower: MpeZoneConfig::lower(3),
            upper: MpeZoneConfig::upper(3),
        });

        let ch = processor.allocate_channel_for_note(60).unwrap();
        // Should prefer lower zone: channels 1-3
        assert!(
            ch >= 1 && ch <= 3,
            "Dual zone should prefer lower zone, got {ch}"
        );
    }

    // -----------------------------------------------------------------------
    // Reset
    // -----------------------------------------------------------------------

    #[test]
    fn test_reset_clears_all_state() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        // Set up some state
        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 100));
        processor.process_midi1(&MidiEvent::pitch_bend(0, 2, 16383));
        processor.process_midi1(&MidiEvent::pitch_bend(0, 0, 12288)); // Global
        processor.allocate_channel_for_note(64);

        // Verify state exists
        assert!(processor.expression().is_active(60));
        assert!(processor.get_channel_for_note(64).is_some());
        assert!(processor.expression().get_pitch_bend_global() > 0.0);

        // Reset
        processor.reset();

        // All state should be cleared
        assert!(!processor.expression().is_active(60));
        assert!(processor.get_channel_for_note(64).is_none());
        assert!((processor.expression().get_pitch_bend_global()).abs() < 0.001);
        assert!((processor.expression().get_pitch_bend_per_note(60)).abs() < 0.001);
    }

    // -----------------------------------------------------------------------
    // CC74 Slide and ChannelPressure routing
    // -----------------------------------------------------------------------

    #[test]
    fn test_cc74_slide_routes_to_note() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        // Note on channel 3
        processor.process_midi1(&MidiEvent::note_on(0, 3, 60, 100));

        // CC74 on channel 3 → slide for note 60
        processor.process_midi1(&MidiEvent::control_change(0, 3, 74, 127));
        let slide = processor.expression().get_slide(60);
        assert!((slide - 1.0).abs() < 0.01);

        // CC74 on master channel (0) → should NOT set slide (CC74 only for members)
        processor.process_midi1(&MidiEvent::control_change(0, 0, 74, 0));
        // Note 60 slide should still be 1.0
        assert!((processor.expression().get_slide(60) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_channel_pressure_master_vs_member() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 100));
        processor.process_midi1(&MidiEvent::note_on(0, 3, 72, 100));

        // Member pressure (ch 2) → per-note for note 60
        processor.process_midi1(&MidiEvent::aftertouch(0, 2, 100));
        let p60 = processor.expression().get_pressure_per_note(60);
        let p72 = processor.expression().get_pressure_per_note(72);
        assert!((p60 - 100.0 / 127.0).abs() < 0.01);
        assert!(
            (p72).abs() < 0.01,
            "Note 72 should have no per-note pressure"
        );

        // Master pressure (ch 0) → global, affects both via max()
        processor.process_midi1(&MidiEvent::aftertouch(0, 0, 64));
        let p60 = processor.expression().get_pressure(60);
        let p72 = processor.expression().get_pressure(72);
        // Note 60: max(100/127, 64/127) = 100/127
        assert!((p60 - 100.0 / 127.0).abs() < 0.01);
        // Note 72: max(0, 64/127) = 64/127
        assert!((p72 - 64.0 / 127.0).abs() < 0.01);
    }

    #[test]
    fn test_poly_pressure_routes_directly_to_note() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 100));

        // PolyPressure specifies note directly (no channel mapping needed)
        processor.process_midi1(&MidiEvent::poly_aftertouch(0, 2, 60, 100));
        let pressure = processor.expression().get_pressure_per_note(60);
        assert!((pressure - 100.0 / 127.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Note Off clears channel mapping
    // -----------------------------------------------------------------------

    #[test]
    fn test_note_off_clears_channel_mapping() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 100));

        // Verify channel mapping exists
        let map = processor.lower_zone_map.as_ref().unwrap();
        assert_eq!(map.get_note_for_channel(2), Some(60));
        assert_eq!(map.get_channel_for_note(60), Some(2));

        // Note off
        processor.process_midi1(&MidiEvent::note_off(0, 2, 60, 0));

        // Channel mapping should be cleared
        let map = processor.lower_zone_map.as_ref().unwrap();
        assert!(map.get_note_for_channel(2).is_none());
        assert!(map.get_channel_for_note(60).is_none());
    }

    #[test]
    fn test_velocity_zero_note_on_clears_mapping() {
        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5)));

        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 100));
        // NoteOn velocity=0 should act as NoteOff
        processor.process_midi1(&MidiEvent::note_on(0, 2, 60, 0));

        let map = processor.lower_zone_map.as_ref().unwrap();
        assert!(map.get_note_for_channel(2).is_none());
        assert!(map.get_channel_for_note(60).is_none());
        assert!(!processor.expression().is_active(60));
    }

    // -----------------------------------------------------------------------
    // MIDI 2.0 tests
    // -----------------------------------------------------------------------

    #[cfg(feature = "midi2")]
    #[test]
    fn test_midi2_channel_pitch_bend() {
        use midi2::prelude::*;

        let processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        // Channel pitch bend (global)
        let bend = crate::midi2::Midi2Event::channel_pitch_bend(
            0,
            u4::new(0),
            u4::new(0),
            0xFFFFFFFF, // Max
        );
        processor.process_midi2(&bend);

        let global = processor.expression().get_pitch_bend_global();
        assert!((global - 1.0).abs() < 0.01);
    }

    #[cfg(feature = "midi2")]
    #[test]
    fn test_midi2_key_pressure() {
        use midi2::prelude::*;

        let processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        processor.process_midi2(&crate::midi2::Midi2Event::note_on(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            65535,
        ));

        let pressure = crate::midi2::Midi2Event::key_pressure(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            0xFFFFFFFF,
        );
        processor.process_midi2(&pressure);

        let p = processor.expression().get_pressure_per_note(60);
        assert!((p - 1.0).abs() < 0.01);
    }

    #[cfg(feature = "midi2")]
    #[test]
    fn test_midi2_assignable_per_note_controller_cc74() {
        use midi2::prelude::*;

        let processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        processor.process_midi2(&crate::midi2::Midi2Event::note_on(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            65535,
        ));

        // Construct Assignable Per-Note Controller UMP manually:
        // opcode=0x1, group=0, channel=0, note=60, index=74, value=0xFFFFFFFF
        // Word 0: 0x4 (type) | 0x1 (opcode) | 0x0 (channel) | 60 (note) | 74 (index)
        //       = 0x4010_3C4A
        let cc74 = crate::midi2::Midi2Event::try_from_ump(0, &[0x4010_3C4A, 0xFFFFFFFF]).unwrap();
        processor.process_midi2(&cc74);

        let slide = processor.expression().get_slide(60);
        assert!((slide - 1.0).abs() < 0.01);
    }

    #[cfg(feature = "midi2")]
    #[test]
    fn test_process_unified_dispatches_correctly() {
        use midi2::prelude::*;

        let mut processor = MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15)));

        // V1 event
        let v1 = crate::event::UnifiedMidiEvent::V1(MidiEvent::note_on(0, 2, 60, 100));
        processor.process_unified(&v1);
        assert!(processor.expression().is_active(60));

        // V2 event
        let v2_note =
            crate::midi2::Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(72), 65535);
        let v2 = crate::event::UnifiedMidiEvent::V2(v2_note);
        processor.process_unified(&v2);
        assert!(processor.expression().is_active(72));
    }
}
