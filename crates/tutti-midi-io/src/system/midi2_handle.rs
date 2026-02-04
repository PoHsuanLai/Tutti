//! MIDI 2.0 sub-handle for high-resolution messages.

use crate::event::MidiEvent;
use crate::midi2::Midi2Event;

/// Handle for MIDI 2.0 functionality
pub struct Midi2Handle;

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
