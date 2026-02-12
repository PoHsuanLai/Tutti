//! RT-safe MIDI event types with sample-accurate timing.
//!
//! Core types (`MidiEvent`, `RawMidiEvent`, `MidiEventBuilder`) are defined in
//! `tutti-core` and re-exported here. This module adds MIDI 2.0 extensions.

pub use tutti_midi::{MidiEvent, RawMidiEvent};

#[cfg(test)]
pub(crate) use tutti_midi::ControlChange;

#[cfg(any(feature = "midi2", test))]
use tutti_midi::ChannelVoiceMsg;

/// Upscale 7-bit MIDI 1.0 velocity to 16-bit MIDI 2.0 range.
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

#[cfg(feature = "midi2")]
#[inline]
pub fn to_midi2(event: &MidiEvent) -> Option<super::midi2::Midi2Event> {
    super::midi2::midi1_to_midi2(event)
}

/// Unified MIDI 1.0/2.0 event.
#[cfg(feature = "midi2")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UnifiedMidiEvent {
    V1(MidiEvent),
    V2(super::midi2::Midi2Event),
}

#[cfg(feature = "midi2")]
impl UnifiedMidiEvent {
    #[inline]
    pub fn frame_offset(&self) -> usize {
        match self {
            UnifiedMidiEvent::V1(e) => e.frame_offset,
            UnifiedMidiEvent::V2(e) => e.frame_offset,
        }
    }

    #[inline]
    pub fn channel(&self) -> u8 {
        match self {
            UnifiedMidiEvent::V1(e) => e.channel_num(),
            UnifiedMidiEvent::V2(e) => e.channel(),
        }
    }

    #[inline]
    pub fn is_note_on(&self) -> bool {
        match self {
            UnifiedMidiEvent::V1(e) => e.is_note_on(),
            UnifiedMidiEvent::V2(e) => e.is_note_on(),
        }
    }

    #[inline]
    pub fn is_note_off(&self) -> bool {
        match self {
            UnifiedMidiEvent::V1(e) => e.is_note_off(),
            UnifiedMidiEvent::V2(e) => e.is_note_off(),
        }
    }

    #[inline]
    pub fn note(&self) -> Option<u8> {
        match self {
            UnifiedMidiEvent::V1(e) => e.note(),
            UnifiedMidiEvent::V2(e) => e.note(),
        }
    }

    /// Velocity scaled to 0.0..=1.0.
    #[inline]
    pub fn velocity_normalized(&self) -> Option<f32> {
        match self {
            UnifiedMidiEvent::V1(e) => e.velocity().map(|v| v as f32 / 127.0),
            UnifiedMidiEvent::V2(e) => e.velocity_normalized(),
        }
    }

    /// Velocity as 7-bit value (0-127).
    #[inline]
    pub fn velocity(&self) -> Option<u8> {
        match self {
            UnifiedMidiEvent::V1(e) => e.velocity(),
            UnifiedMidiEvent::V2(e) => e.velocity(),
        }
    }

    /// Velocity as 16-bit value (0-65535).
    #[inline]
    pub fn velocity_16bit(&self) -> Option<u16> {
        match self {
            UnifiedMidiEvent::V1(e) => velocity_16bit(e),
            UnifiedMidiEvent::V2(e) => e.velocity_16bit(),
        }
    }

    #[inline]
    pub fn to_midi1(&self) -> Option<MidiEvent> {
        match self {
            UnifiedMidiEvent::V1(e) => Some(*e),
            UnifiedMidiEvent::V2(e) => e.to_midi1(),
        }
    }

    #[inline]
    pub fn to_midi2(&self) -> Option<super::midi2::Midi2Event> {
        match self {
            UnifiedMidiEvent::V1(e) => to_midi2(e),
            UnifiedMidiEvent::V2(e) => Some(*e),
        }
    }

    #[inline]
    pub fn is_v1(&self) -> bool {
        matches!(self, UnifiedMidiEvent::V1(_))
    }

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

    // Base MidiEvent tests live in tutti-midi. Only MIDI 2.0 extension tests here.

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
