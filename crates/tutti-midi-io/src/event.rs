//! RT-safe MIDI event types with sample-accurate timing.
//!
//! Core types (`MidiEvent`, `RawMidiEvent`, `MidiEventBuilder`) are defined in
//! `tutti-core` and re-exported here. This module adds MIDI 2.0 extensions.

// Re-export core MIDI types from tutti-midi
pub use tutti_midi::{MidiEvent, RawMidiEvent};

// Re-export ControlChange for tests
#[cfg(test)]
pub(crate) use tutti_midi::ControlChange;

// Import ChannelVoiceMsg for midi2 feature functions
#[cfg(feature = "midi2")]
use tutti_midi::ChannelVoiceMsg;

/// Get velocity as 16-bit MIDI 2.0 value
#[cfg(feature = "midi2")]
#[inline]
pub fn velocity_16bit(event: &MidiEvent) -> Option<u16> {
    match event.msg {
        ChannelVoiceMsg::NoteOn { velocity, .. } | ChannelVoiceMsg::NoteOff { velocity, .. } => {
            Some(super::midi2::midi1_velocity_to_midi2(velocity))
        }
        ChannelVoiceMsg::HighResNoteOn { velocity, .. }
        | ChannelVoiceMsg::HighResNoteOff { velocity, .. } => {
            // High-res velocity is 14-bit, scale to 16-bit
            Some(((velocity as u32 * 65535) / 16383) as u16)
        }
        _ => None,
    }
}

/// Convert MIDI 1.0 event to MIDI 2.0 event
#[cfg(feature = "midi2")]
#[inline]
pub fn to_midi2(event: &MidiEvent) -> Option<super::midi2::Midi2Event> {
    super::midi2::midi1_to_midi2(event)
}

/// Unified MIDI 1.0/2.0 event.
#[cfg(feature = "midi2")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnifiedMidiEvent {
    /// MIDI 1.0 channel voice message
    V1(MidiEvent),
    /// MIDI 2.0 channel voice message
    V2(super::midi2::Midi2Event),
}

#[cfg(feature = "midi2")]
impl UnifiedMidiEvent {
    /// Get the frame offset
    #[inline]
    pub fn frame_offset(&self) -> usize {
        match self {
            UnifiedMidiEvent::V1(e) => e.frame_offset,
            UnifiedMidiEvent::V2(e) => e.frame_offset,
        }
    }

    /// Get the MIDI channel
    #[inline]
    pub fn channel(&self) -> u8 {
        match self {
            UnifiedMidiEvent::V1(e) => e.channel_num(),
            UnifiedMidiEvent::V2(e) => e.channel(),
        }
    }

    /// Check if this is a note on event
    #[inline]
    pub fn is_note_on(&self) -> bool {
        match self {
            UnifiedMidiEvent::V1(e) => e.is_note_on(),
            UnifiedMidiEvent::V2(e) => e.is_note_on(),
        }
    }

    /// Check if this is a note off event
    #[inline]
    pub fn is_note_off(&self) -> bool {
        match self {
            UnifiedMidiEvent::V1(e) => e.is_note_off(),
            UnifiedMidiEvent::V2(e) => e.is_note_off(),
        }
    }

    /// Get the note number
    #[inline]
    pub fn note(&self) -> Option<u8> {
        match self {
            UnifiedMidiEvent::V1(e) => e.note(),
            UnifiedMidiEvent::V2(e) => e.note(),
        }
    }

    /// Get velocity as normalized f32
    #[inline]
    pub fn velocity_normalized(&self) -> Option<f32> {
        match self {
            UnifiedMidiEvent::V1(e) => e.velocity().map(|v| v as f32 / 127.0),
            UnifiedMidiEvent::V2(e) => e.velocity_normalized(),
        }
    }

    /// Get velocity as 7-bit value
    #[inline]
    pub fn velocity(&self) -> Option<u8> {
        match self {
            UnifiedMidiEvent::V1(e) => e.velocity(),
            UnifiedMidiEvent::V2(e) => e.velocity(),
        }
    }

    /// Get velocity as 16-bit value
    #[inline]
    pub fn velocity_16bit(&self) -> Option<u16> {
        match self {
            UnifiedMidiEvent::V1(e) => velocity_16bit(e),
            UnifiedMidiEvent::V2(e) => e.velocity_16bit(),
        }
    }

    /// Convert to MIDI 1.0 event
    #[inline]
    pub fn to_midi1(&self) -> Option<MidiEvent> {
        match self {
            UnifiedMidiEvent::V1(e) => Some(*e),
            UnifiedMidiEvent::V2(e) => e.to_midi1(),
        }
    }

    /// Convert to MIDI 2.0 event
    #[inline]
    pub fn to_midi2(&self) -> Option<super::midi2::Midi2Event> {
        match self {
            UnifiedMidiEvent::V1(e) => to_midi2(e),
            UnifiedMidiEvent::V2(e) => Some(*e),
        }
    }

    /// Check if this is a MIDI 1.0 event.
    #[inline]
    pub fn is_v1(&self) -> bool {
        matches!(self, UnifiedMidiEvent::V1(_))
    }

    /// Check if this is a MIDI 2.0 event.
    #[inline]
    pub fn is_v2(&self) -> bool {
        matches!(self, UnifiedMidiEvent::V2(_))
    }
}

#[cfg(feature = "midi2")]
impl From<MidiEvent> for UnifiedMidiEvent {
    fn from(event: MidiEvent) -> Self {
        UnifiedMidiEvent::V1(event)
    }
}

#[cfg(feature = "midi2")]
impl From<super::midi2::Midi2Event> for UnifiedMidiEvent {
    fn from(event: super::midi2::Midi2Event) -> Self {
        UnifiedMidiEvent::V2(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_on() {
        let event = MidiEvent::note_on(100, 0, 60, 100);
        assert!(event.is_note_on());
        assert!(!event.is_note_off());
        assert_eq!(event.note(), Some(60));
        assert_eq!(event.velocity(), Some(100));
        assert_eq!(event.channel_num(), 0);
        assert_eq!(event.frame_offset, 100);
    }

    #[test]
    fn test_note_off() {
        let event = MidiEvent::note_off(50, 3, 64, 0);
        assert!(event.is_note_off());
        assert!(!event.is_note_on());
        assert_eq!(event.note(), Some(64));
        assert_eq!(event.channel_num(), 3);
    }

    #[test]
    fn test_note_on_zero_velocity_is_note_off() {
        let event = MidiEvent::note_on(0, 0, 60, 0);
        assert!(event.is_note_off());
        assert!(!event.is_note_on());
    }

    #[test]
    fn test_control_change() {
        let event = MidiEvent::control_change(0, 5, 7, 127);
        assert_eq!(event.channel_num(), 5);
        match event.msg {
            ChannelVoiceMsg::ControlChange { control } => match control {
                ControlChange::CC { control: cc, value } => {
                    assert_eq!(cc, 7);
                    assert_eq!(value, 127);
                }
                _ => panic!("Expected CC"),
            },
            _ => panic!("Expected ControlChange"),
        }
    }

    #[test]
    fn test_pitch_bend() {
        let event = MidiEvent::pitch_bend(0, 0, 8192); // Center
        match event.msg {
            ChannelVoiceMsg::PitchBend { bend } => {
                assert_eq!(bend, 8192);
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[test]
    fn test_serialization_roundtrip() {
        let event = MidiEvent::note_on(0, 5, 60, 100);
        let bytes = event.to_bytes();
        let parsed = MidiEvent::from_bytes(&bytes).unwrap();
        assert_eq!(event.channel, parsed.channel);
        assert_eq!(event.msg, parsed.msg);
    }

    #[test]
    fn test_raw_midi_event() {
        let event = MidiEvent::note_on(100, 0, 60, 100);
        let raw: RawMidiEvent = event.into();
        assert_eq!(raw.frame_offset, 100);
        assert_eq!(raw.status(), 0x90);
        assert_eq!(raw.channel(), 0);

        let back = raw.to_midi_event().unwrap();
        assert_eq!(back.channel, event.channel);
        assert_eq!(back.msg, event.msg);
    }

    #[test]
    fn test_builder_simple() {
        let event = MidiEvent::note_on_builder(60, 100).build();
        assert_eq!(event.note(), Some(60));
        assert_eq!(event.velocity(), Some(100));
        assert_eq!(event.channel_num(), 0);
        assert_eq!(event.frame_offset, 0);
    }

    #[test]
    fn test_builder_with_channel() {
        let event = MidiEvent::note_on_builder(64, 80).channel(5).build();
        assert_eq!(event.note(), Some(64));
        assert_eq!(event.velocity(), Some(80));
        assert_eq!(event.channel_num(), 5);
        assert_eq!(event.frame_offset, 0);
    }

    #[test]
    fn test_builder_with_offset() {
        let event = MidiEvent::note_on_builder(67, 120).offset(480).build();
        assert_eq!(event.note(), Some(67));
        assert_eq!(event.velocity(), Some(120));
        assert_eq!(event.channel_num(), 0);
        assert_eq!(event.frame_offset, 480);
    }

    #[test]
    fn test_builder_with_both() {
        let event = MidiEvent::note_on_builder(71, 90)
            .channel(3)
            .offset(960)
            .build();
        assert_eq!(event.note(), Some(71));
        assert_eq!(event.velocity(), Some(90));
        assert_eq!(event.channel_num(), 3);
        assert_eq!(event.frame_offset, 960);
    }

    #[test]
    fn test_builder_cc() {
        let event = MidiEvent::cc_builder(7, 127).channel(2).build();
        assert_eq!(event.channel_num(), 2);
        match event.msg {
            ChannelVoiceMsg::ControlChange { control } => match control {
                ControlChange::CC { control: cc, value } => {
                    assert_eq!(cc, 7);
                    assert_eq!(value, 127);
                }
                _ => panic!("Expected CC"),
            },
            _ => panic!("Expected ControlChange"),
        }
    }

    #[test]
    fn test_builder_pitch_bend() {
        let event = MidiEvent::bend_builder(8192).build();
        match event.msg {
            ChannelVoiceMsg::PitchBend { bend } => {
                assert_eq!(bend, 8192);
            }
            _ => panic!("Expected PitchBend"),
        }
    }

    #[cfg(feature = "midi2")]
    mod midi2_tests {
        use super::*;

        #[test]
        fn test_velocity_normalized() {
            let event = MidiEvent::note_on(0, 0, 60, 127);
            let norm = event.velocity().map(|v| v as f32 / 127.0).unwrap();
            assert!((norm - 1.0).abs() < 0.01);

            let event = MidiEvent::note_on(0, 0, 60, 0);
            let norm = event.velocity().map(|v| v as f32 / 127.0).unwrap();
            assert!((norm - 0.0).abs() < 0.01);

            let event = MidiEvent::note_on(0, 0, 60, 64);
            let norm = event.velocity().map(|v| v as f32 / 127.0).unwrap();
            assert!((norm - 0.5).abs() < 0.02);
        }

        #[test]
        fn test_velocity_16bit() {
            let event = MidiEvent::note_on(0, 0, 60, 127);
            let vel16 = velocity_16bit(&event).unwrap();
            assert_eq!(vel16, 65535);

            let event = MidiEvent::note_on(0, 0, 60, 0);
            let vel16 = velocity_16bit(&event).unwrap();
            assert_eq!(vel16, 0);
        }

        #[test]
        fn test_to_midi2_conversion() {
            let event = MidiEvent::note_on(100, 5, 60, 100);
            let midi2 = to_midi2(&event).unwrap();

            assert_eq!(midi2.frame_offset, 100);
            assert_eq!(midi2.channel(), 5);
            assert!(midi2.is_note_on());
            assert_eq!(midi2.note(), Some(60));
        }

        #[test]
        fn test_unified_event_from_v1() {
            let v1 = MidiEvent::note_on(50, 3, 64, 80);
            let unified: UnifiedMidiEvent = v1.into();

            assert!(unified.is_v1());
            assert!(!unified.is_v2());
            assert_eq!(unified.frame_offset(), 50);
            assert_eq!(unified.channel(), 3);
            assert!(unified.is_note_on());
            assert_eq!(unified.note(), Some(64));
            assert_eq!(unified.velocity(), Some(80));
        }

        #[test]
        fn test_unified_event_from_v2() {
            use midi2::prelude::*;
            let v2 =
                crate::midi2::Midi2Event::note_on(100, u4::new(0), u4::new(5), u7::new(60), 32768);
            let unified: UnifiedMidiEvent = v2.into();

            assert!(!unified.is_v1());
            assert!(unified.is_v2());
            assert_eq!(unified.frame_offset(), 100);
            assert_eq!(unified.channel(), 5);
            assert!(unified.is_note_on());
            assert_eq!(unified.note(), Some(60));
        }

        #[test]
        fn test_unified_velocity_normalized() {
            // MIDI 1.0 event
            let v1 = MidiEvent::note_on(0, 0, 60, 127);
            let unified: UnifiedMidiEvent = v1.into();
            let norm = unified.velocity_normalized().unwrap();
            assert!((norm - 1.0).abs() < 0.01);

            // MIDI 2.0 event
            use midi2::prelude::*;
            let v2 =
                crate::midi2::Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 65535);
            let unified: UnifiedMidiEvent = v2.into();
            let norm = unified.velocity_normalized().unwrap();
            assert!((norm - 1.0).abs() < 0.0001);
        }

        #[test]
        fn test_unified_to_midi1() {
            // V1 -> V1: should be identical
            let v1 = MidiEvent::note_on(50, 3, 64, 80);
            let unified: UnifiedMidiEvent = v1.into();
            let back = unified.to_midi1().unwrap();
            assert_eq!(back, v1);

            // V2 -> V1: should convert with downsampled velocity
            use midi2::prelude::*;
            let v2 = crate::midi2::Midi2Event::note_on(
                100,
                u4::new(0),
                u4::new(5),
                u7::new(60),
                crate::midi2::midi1_velocity_to_midi2(100),
            );
            let unified: UnifiedMidiEvent = v2.into();
            let v1 = unified.to_midi1().unwrap();
            assert_eq!(v1.note(), Some(60));
            assert_eq!(v1.velocity(), Some(100));
        }
    }
}
