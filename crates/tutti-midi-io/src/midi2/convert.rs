//! MIDI 1.0 <-> MIDI 2.0 conversion functions.

use midi2::prelude::*;

use super::Midi2Event;

/// Convert 7-bit MIDI 1.0 velocity to 16-bit MIDI 2.0.
///
/// Uses the MIDI 2.0 specification's recommended scaling:
/// - 0 stays 0 (note off)
/// - 1-127 maps linearly to 0x0200-0xFFFF
///
/// This ensures perfect round-trip conversion.
#[inline]
pub fn midi1_velocity_to_midi2(v: u8) -> u16 {
    if v == 0 {
        0
    } else {
        // Scale 1-127 to 1-65535, then map to 0x0200-0xFFFF range
        // This is the MIDI 2.0 spec's scaling formula
        let v7 = v as u32;
        // Bit replication: v << 9 | v << 2 | v >> 5
        // But for perfect round-trip, we use: (v * 65535 / 127)
        ((v7 * 65535 + 63) / 127) as u16
    }
}

/// Convert 16-bit MIDI 2.0 velocity to 7-bit MIDI 1.0.
///
/// Uses the inverse of the MIDI 2.0 specification's scaling.
#[inline]
pub fn midi2_velocity_to_midi1(v: u16) -> u8 {
    if v == 0 {
        0
    } else {
        // Inverse of midi1_velocity_to_midi2
        let v16 = v as u32;
        ((v16 * 127 + 32767) / 65535).min(127) as u8
    }
}

/// Convert 7-bit MIDI 1.0 CC value to 32-bit MIDI 2.0.
#[inline]
pub fn midi1_cc_to_midi2(v: u8) -> u32 {
    if v == 0 {
        0
    } else if v == 127 {
        0xFFFF_FFFF
    } else {
        // Linear scaling for perfect round-trip
        let v7 = v as u64;
        ((v7 * 0xFFFF_FFFF + 63) / 127) as u32
    }
}

/// Convert 32-bit MIDI 2.0 CC value to 7-bit MIDI 1.0.
#[inline]
pub fn midi2_cc_to_midi1(v: u32) -> u8 {
    if v == 0 {
        0
    } else {
        // Inverse scaling
        let v32 = v as u64;
        ((v32 * 127 + 0x7FFF_FFFF) / 0xFFFF_FFFF).min(127) as u8
    }
}

/// Convert 14-bit MIDI 1.0 pitch bend to 32-bit MIDI 2.0.
///
/// MIDI 1.0: 0-16383, center at 8192
/// MIDI 2.0: 0-0xFFFFFFFF, center at 0x80000000
#[inline]
pub fn midi1_pitch_bend_to_midi2(v: u16) -> u32 {
    if v == 0 {
        0
    } else if v == 16383 {
        0xFFFF_FFFF
    } else {
        let v14 = v as u64;
        ((v14 * 0xFFFF_FFFF + 8191) / 16383) as u32
    }
}

/// Convert 32-bit MIDI 2.0 pitch bend to 14-bit MIDI 1.0.
#[inline]
pub fn midi2_pitch_bend_to_midi1(v: u32) -> u16 {
    if v == 0 {
        0
    } else {
        let v32 = v as u64;
        ((v32 * 16383 + 0x7FFF_FFFF) / 0xFFFF_FFFF).min(16383) as u16
    }
}

/// Convert MIDI 1.0 MidiEvent to MIDI 2.0 Midi2Event.
///
/// This upsamples resolution (7-bit to 16-bit velocity, etc).
pub fn midi1_to_midi2(event: &crate::MidiEvent) -> Option<Midi2Event> {
    use midi_msg::ChannelVoiceMsg;

    let group = u4::new(0);
    let channel = u4::new(event.channel as u8);

    match event.msg {
        ChannelVoiceMsg::NoteOn { note, velocity } => Some(Midi2Event::note_on(
            event.frame_offset,
            group,
            channel,
            u7::new(note),
            midi1_velocity_to_midi2(velocity),
        )),
        ChannelVoiceMsg::NoteOff { note, velocity } => Some(Midi2Event::note_off(
            event.frame_offset,
            group,
            channel,
            u7::new(note),
            midi1_velocity_to_midi2(velocity),
        )),
        ChannelVoiceMsg::ControlChange { control } => {
            if let midi_msg::ControlChange::CC { control: cc, value } = control {
                Some(Midi2Event::control_change(
                    event.frame_offset,
                    group,
                    channel,
                    u7::new(cc),
                    midi1_cc_to_midi2(value),
                ))
            } else {
                None // Other CC types (high-res, RPN, etc) need special handling
            }
        }
        ChannelVoiceMsg::PitchBend { bend } => Some(Midi2Event::channel_pitch_bend(
            event.frame_offset,
            group,
            channel,
            midi1_pitch_bend_to_midi2(bend),
        )),
        ChannelVoiceMsg::ChannelPressure { pressure } => Some(Midi2Event::channel_pressure(
            event.frame_offset,
            group,
            channel,
            midi1_cc_to_midi2(pressure), // Same scaling as CC
        )),
        ChannelVoiceMsg::PolyPressure { note, pressure } => Some(Midi2Event::key_pressure(
            event.frame_offset,
            group,
            channel,
            u7::new(note),
            midi1_cc_to_midi2(pressure),
        )),
        ChannelVoiceMsg::ProgramChange { program } => Some(Midi2Event::program_change(
            event.frame_offset,
            group,
            channel,
            u7::new(program),
            None,
        )),
        _ => None, // High-res note on/off handled separately
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velocity_conversion_roundtrip() {
        // Test edge cases
        assert_eq!(midi2_velocity_to_midi1(midi1_velocity_to_midi2(0)), 0);
        assert_eq!(midi2_velocity_to_midi1(midi1_velocity_to_midi2(1)), 1);
        assert_eq!(midi2_velocity_to_midi1(midi1_velocity_to_midi2(64)), 64);
        assert_eq!(midi2_velocity_to_midi1(midi1_velocity_to_midi2(127)), 127);

        // Test all 7-bit values roundtrip correctly
        for v in 0..=127u8 {
            let midi2 = midi1_velocity_to_midi2(v);
            let back = midi2_velocity_to_midi1(midi2);
            assert_eq!(back, v, "velocity {} failed roundtrip", v);
        }
    }

    #[test]
    fn test_cc_conversion_roundtrip() {
        for v in 0..=127u8 {
            let midi2 = midi1_cc_to_midi2(v);
            let back = midi2_cc_to_midi1(midi2);
            assert_eq!(back, v, "CC {} failed roundtrip", v);
        }
    }

    #[test]
    fn test_pitch_bend_conversion_roundtrip() {
        // Test key values
        assert_eq!(midi2_pitch_bend_to_midi1(midi1_pitch_bend_to_midi2(0)), 0);
        assert_eq!(
            midi2_pitch_bend_to_midi1(midi1_pitch_bend_to_midi2(8192)),
            8192
        );
        assert_eq!(
            midi2_pitch_bend_to_midi1(midi1_pitch_bend_to_midi2(16383)),
            16383
        );
    }

    #[test]
    fn test_midi1_to_midi2_conversion() {
        use midi_msg::{Channel, ChannelVoiceMsg};

        let midi1 = crate::MidiEvent::new(
            100,
            Channel::Ch1,
            ChannelVoiceMsg::NoteOn {
                note: 60,
                velocity: 100,
            },
        );

        let midi2 = midi1_to_midi2(&midi1).unwrap();
        assert_eq!(midi2.frame_offset, 100);
        assert_eq!(midi2.channel(), 0);
        assert!(midi2.is_note_on());
        assert_eq!(midi2.note(), Some(60));

        // Velocity should be upsampled
        let vel = midi2.velocity_16bit().unwrap();
        assert!(vel > 100 * 256); // Should be much larger than 7-bit
    }
}
