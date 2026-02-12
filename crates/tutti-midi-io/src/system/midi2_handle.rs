//! MIDI 2.0 sub-handle for high-resolution messages.

use crate::event::MidiEvent;
use crate::midi2::Midi2Event;

pub struct Midi2Handle;

impl Midi2Handle {
    /// `velocity` is normalized 0.0..1.0 (mapped to 16-bit internally).
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

    /// `bend` is normalized -1.0..1.0 (0.0 = center), mapped to 32-bit internally.
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

    pub fn channel_pitch_bend(&self, bend: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let bend_clamped = bend.clamp(-1.0, 1.0);
        let bend32 = ((bend_clamped as f64 + 1.0) * 0x80000000_u32 as f64) as u32;
        Midi2Event::channel_pitch_bend(0, u4::new(0), u4::new(channel.min(15)), bend32)
    }

    /// `value` is normalized 0.0..1.0 (mapped to 32-bit internally).
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

    pub fn channel_pressure(&self, pressure: f32, channel: u8) -> Midi2Event {
        use midi2::prelude::*;
        let press32 = (pressure.clamp(0.0, 1.0) as f64 * 0xFFFFFFFF_u32 as f64) as u32;
        Midi2Event::channel_pressure(0, u4::new(0), u4::new(channel.min(15)), press32)
    }

    /// Upsamples resolution (7/14-bit to 16/32-bit).
    pub fn convert_to_midi2(&self, event: &MidiEvent) -> Option<Midi2Event> {
        crate::midi2::midi1_to_midi2(event)
    }

    /// Downsamples resolution (16/32-bit to 7/14-bit).
    pub fn convert_to_midi1(&self, event: &Midi2Event) -> Option<MidiEvent> {
        event.to_midi1()
    }

    #[inline]
    pub fn velocity_to_16bit(&self, v: u8) -> u16 {
        crate::midi2::midi1_velocity_to_midi2(v)
    }

    #[inline]
    pub fn velocity_to_7bit(&self, v: u16) -> u8 {
        crate::midi2::midi2_velocity_to_midi1(v)
    }

    #[inline]
    pub fn cc_to_32bit(&self, v: u8) -> u32 {
        crate::midi2::midi1_cc_to_midi2(v)
    }

    #[inline]
    pub fn cc_to_7bit(&self, v: u32) -> u8 {
        crate::midi2::midi2_cc_to_midi1(v)
    }

    #[inline]
    pub fn pitch_bend_to_32bit(&self, v: u16) -> u32 {
        crate::midi2::midi1_pitch_bend_to_midi2(v)
    }

    #[inline]
    pub fn pitch_bend_to_14bit(&self, v: u32) -> u16 {
        crate::midi2::midi2_pitch_bend_to_midi1(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi2::Midi2MessageType;

    #[test]
    fn test_note_on_velocity_clamping() {
        let handle = Midi2Handle;

        // Normal range
        let event = handle.note_on(60, 0.5, 0);
        let vel = event.velocity_16bit().unwrap();
        assert!(vel > 32000 && vel < 33500, "0.5 should map to ~32768, got {vel}");

        // Clamped: negative velocity → 0
        let event = handle.note_on(60, -1.0, 0);
        assert_eq!(event.velocity_16bit().unwrap(), 0);

        // Clamped: over 1.0 → max
        let event = handle.note_on(60, 2.0, 0);
        assert_eq!(event.velocity_16bit().unwrap(), 65535);
    }

    #[test]
    fn test_note_on_channel_clamping() {
        let handle = Midi2Handle;

        // Channel 15 is valid max
        let event = handle.note_on(60, 0.8, 15);
        assert_eq!(event.channel(), 15);

        // Channel 16+ should be clamped to 15
        let event = handle.note_on(60, 0.8, 20);
        assert_eq!(event.channel(), 15);

        let event = handle.note_on(60, 0.8, 255);
        assert_eq!(event.channel(), 15);
    }

    #[test]
    fn test_per_note_pitch_bend_range() {
        let handle = Midi2Handle;

        // Bend -1.0 → should be near 0
        let event = handle.per_note_pitch_bend(60, -1.0, 0);
        if let Midi2MessageType::PerNotePitchBend { bend, .. } = event.message_type() {
            assert!(bend < 100, "bend -1.0 should map near 0, got {bend}");
        } else {
            panic!("Expected PerNotePitchBend");
        }

        // Bend 0.0 → center (~0x80000000)
        let event = handle.per_note_pitch_bend(60, 0.0, 0);
        if let Midi2MessageType::PerNotePitchBend { bend, .. } = event.message_type() {
            let center = 0x80000000_u32;
            assert!(
                (bend as i64 - center as i64).unsigned_abs() < 100,
                "bend 0.0 should be near center, got {bend}"
            );
        } else {
            panic!("Expected PerNotePitchBend");
        }

        // Bend 1.0 → max (~0xFFFFFFFF)
        let event = handle.per_note_pitch_bend(60, 1.0, 0);
        if let Midi2MessageType::PerNotePitchBend { bend, .. } = event.message_type() {
            assert!(bend > 0xFFFF_FF00, "bend 1.0 should be near max, got {bend}");
        } else {
            panic!("Expected PerNotePitchBend");
        }
    }

    #[test]
    fn test_control_change_precision() {
        let handle = Midi2Handle;

        // CC 0.0 → 0
        let event = handle.control_change(74, 0.0, 0);
        if let Midi2MessageType::ControlChange { value, .. } = event.message_type() {
            assert_eq!(value, 0);
        } else {
            panic!("Expected ControlChange");
        }

        // CC 1.0 → max
        let event = handle.control_change(74, 1.0, 0);
        if let Midi2MessageType::ControlChange { value, .. } = event.message_type() {
            assert_eq!(value, 0xFFFFFFFF);
        } else {
            panic!("Expected ControlChange");
        }

        // CC 0.5 → roughly half
        let event = handle.control_change(74, 0.5, 0);
        if let Midi2MessageType::ControlChange { value, .. } = event.message_type() {
            let half = 0x7FFFFFFF_u32;
            assert!(
                (value as i64 - half as i64).unsigned_abs() < 0x1000000,
                "CC 0.5 should be near half, got {value:#X}"
            );
        } else {
            panic!("Expected ControlChange");
        }
    }

    #[test]
    fn test_velocity_roundtrip() {
        let handle = Midi2Handle;

        // Every 7-bit velocity should survive the round-trip
        for v in 0..=127u8 {
            let v16 = handle.velocity_to_16bit(v);
            let v7 = handle.velocity_to_7bit(v16);
            assert_eq!(v7, v, "Velocity round-trip failed for {v}: 7→{v16}→{v7}");
        }
    }
}
