//! MIDI 2.0 event type with sample-accurate timing.

use midi2::channel_voice2::NoteAttribute;
use midi2::prelude::*;

use super::convert::midi2_velocity_to_midi1;
use super::Midi2MessageType;

/// Helper to extract [u32; 2] from a message's data slice
#[inline]
fn data_to_array(data: &[u32]) -> [u32; 2] {
    [data[0], data[1]]
}

/// RT-safe MIDI 2.0 event with sample-accurate timing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Midi2Event {
    /// Sample offset within the current buffer (0 = first sample)
    pub frame_offset: usize,
    /// UMP packet data (64-bit for channel voice 2)
    pub data: [u32; 2],
}

impl Midi2Event {
    // ==================== Constructors ====================

    /// Create a MIDI 2.0 Note On event
    #[inline]
    pub fn note_on(frame: usize, group: u4, channel: u4, note: u7, velocity: u16) -> Self {
        let mut msg = midi2::channel_voice2::NoteOn::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_note_number(note);
        msg.set_velocity(velocity);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Note On event with attribute
    #[inline]
    pub fn note_on_with_attribute(
        frame: usize,
        group: u4,
        channel: u4,
        note: u7,
        velocity: u16,
        attribute_type: NoteAttribute,
    ) -> Self {
        let mut msg = midi2::channel_voice2::NoteOn::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_note_number(note);
        msg.set_velocity(velocity);
        msg.set_attribute(Some(attribute_type));
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Note Off event
    #[inline]
    pub fn note_off(frame: usize, group: u4, channel: u4, note: u7, velocity: u16) -> Self {
        let mut msg = midi2::channel_voice2::NoteOff::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_note_number(note);
        msg.set_velocity(velocity);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Per-Note Pitch Bend event
    #[inline]
    pub fn per_note_pitch_bend(frame: usize, group: u4, channel: u4, note: u7, bend: u32) -> Self {
        let mut msg = midi2::channel_voice2::PerNotePitchBend::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_note_number(note);
        msg.set_pitch_bend_data(bend);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Control Change event
    #[inline]
    pub fn control_change(
        frame: usize,
        group: u4,
        channel: u4,
        controller: u7,
        value: u32,
    ) -> Self {
        let mut msg = midi2::channel_voice2::ControlChange::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_control(controller);
        msg.set_control_change_data(value);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Channel Pitch Bend event
    #[inline]
    pub fn channel_pitch_bend(frame: usize, group: u4, channel: u4, bend: u32) -> Self {
        let mut msg = midi2::channel_voice2::ChannelPitchBend::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_pitch_bend_data(bend);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Channel Pressure event
    #[inline]
    pub fn channel_pressure(frame: usize, group: u4, channel: u4, pressure: u32) -> Self {
        let mut msg = midi2::channel_voice2::ChannelPressure::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_channel_pressure_data(pressure);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Per-Note Pressure event
    #[inline]
    pub fn key_pressure(frame: usize, group: u4, channel: u4, note: u7, pressure: u32) -> Self {
        let mut msg = midi2::channel_voice2::KeyPressure::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_note_number(note);
        msg.set_key_pressure_data(pressure);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    /// Create a MIDI 2.0 Program Change event
    #[inline]
    pub fn program_change(
        frame: usize,
        group: u4,
        channel: u4,
        program: u7,
        bank: Option<u14>,
    ) -> Self {
        let mut msg = midi2::channel_voice2::ProgramChange::<[u32; 2]>::new();
        msg.set_group(group);
        msg.set_channel(channel);
        msg.set_program(program);
        msg.set_bank(bank);
        Self {
            frame_offset: frame,
            data: data_to_array(msg.data()),
        }
    }

    // ==================== Parsing ====================

    /// Try to create a Midi2Event from raw UMP data
    #[inline]
    pub fn try_from_ump(frame: usize, data: &[u32]) -> Option<Self> {
        if data.len() < 2 {
            return None;
        }
        // Check message type nibble (bits 28-31 of first word)
        // Channel Voice 2 = 0x4
        let msg_type = (data[0] >> 28) & 0x0F;
        if msg_type != 0x4 {
            return None;
        }
        Some(Self {
            frame_offset: frame,
            data: [data[0], data[1]],
        })
    }

    // ==================== Accessors ====================

    /// Get the UMP group (0-15).
    #[inline]
    pub fn group(&self) -> u8 {
        ((self.data[0] >> 24) & 0x0F) as u8
    }

    /// Get the MIDI channel (0-15).
    #[inline]
    pub fn channel(&self) -> u8 {
        ((self.data[0] >> 16) & 0x0F) as u8
    }

    /// Get the opcode (message type within channel voice 2).
    #[inline]
    fn opcode(&self) -> u8 {
        ((self.data[0] >> 20) & 0x0F) as u8
    }

    /// Parse the message type and extract relevant data.
    pub fn message_type(&self) -> Midi2MessageType {
        match self.opcode() {
            0x9 => {
                // Note On
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let attr_type = (self.data[0] & 0xFF) as u8;
                let velocity = (self.data[1] >> 16) as u16;
                let attr_data = (self.data[1] & 0xFFFF) as u16;
                let attribute = if attr_type != 0 {
                    Some(attr_data)
                } else {
                    None
                };
                Midi2MessageType::NoteOn {
                    note,
                    velocity,
                    attribute,
                }
            }
            0x8 => {
                // Note Off
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let velocity = (self.data[1] >> 16) as u16;
                Midi2MessageType::NoteOff { note, velocity }
            }
            0x6 => {
                // Per-Note Pitch Bend
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let bend = self.data[1];
                Midi2MessageType::PerNotePitchBend { note, bend }
            }
            0xB => {
                // Control Change
                let controller = ((self.data[0] >> 8) & 0x7F) as u8;
                let value = self.data[1];
                Midi2MessageType::ControlChange { controller, value }
            }
            0xE => {
                // Pitch Bend
                let bend = self.data[1];
                Midi2MessageType::ChannelPitchBend { bend }
            }
            0xD => {
                // Channel Pressure
                let pressure = self.data[1];
                Midi2MessageType::ChannelPressure { pressure }
            }
            0xA => {
                // Poly Pressure
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let pressure = self.data[1];
                Midi2MessageType::KeyPressure { note, pressure }
            }
            0xC => {
                // Program Change
                let program = ((self.data[1] >> 24) & 0x7F) as u8;
                let bank_valid = (self.data[0] >> 31) != 0; // Bit 31 of first word
                let bank = if bank_valid {
                    // Bank is stored in octets 2 and 3 of second word as 7-bit values
                    let msb = ((self.data[1] >> 8) & 0x7F) as u8;
                    let lsb = (self.data[1] & 0x7F) as u8;
                    Some(((msb as u16) << 7) | (lsb as u16))
                } else {
                    None
                };
                Midi2MessageType::ProgramChange { program, bank }
            }
            0x0 => {
                // Registered Per-Note Controller
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let index = (self.data[0] & 0xFF) as u8;
                let value = self.data[1];
                Midi2MessageType::RegisteredPerNoteController { note, index, value }
            }
            0x1 => {
                // Assignable Per-Note Controller
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let index = (self.data[0] & 0xFF) as u8;
                let value = self.data[1];
                Midi2MessageType::AssignablePerNoteController { note, index, value }
            }
            0x2 => {
                // Registered Controller (RPN)
                let bank = ((self.data[0] >> 8) & 0x7F) as u8;
                let index = (self.data[0] & 0x7F) as u8;
                let value = self.data[1];
                Midi2MessageType::RegisteredController { bank, index, value }
            }
            0x3 => {
                // Assignable Controller (NRPN)
                let bank = ((self.data[0] >> 8) & 0x7F) as u8;
                let index = (self.data[0] & 0x7F) as u8;
                let value = self.data[1];
                Midi2MessageType::AssignableController { bank, index, value }
            }
            0xF => {
                // Per-Note Management
                let note = ((self.data[0] >> 8) & 0x7F) as u8;
                let detach = (self.data[0] & 0x02) != 0;
                let reset = (self.data[0] & 0x01) != 0;
                Midi2MessageType::PerNoteManagement {
                    note,
                    detach,
                    reset,
                }
            }
            _ => Midi2MessageType::Unknown {
                opcode: self.opcode(),
            },
        }
    }

    /// Check if this is a note on event.
    #[inline]
    pub fn is_note_on(&self) -> bool {
        matches!(self.message_type(), Midi2MessageType::NoteOn { velocity, .. } if velocity > 0)
    }

    /// Check if this is a note off event.
    #[inline]
    pub fn is_note_off(&self) -> bool {
        matches!(
            self.message_type(),
            Midi2MessageType::NoteOff { .. } | Midi2MessageType::NoteOn { velocity: 0, .. }
        )
    }

    /// Get note number (for note-related events).
    #[inline]
    pub fn note(&self) -> Option<u8> {
        match self.message_type() {
            Midi2MessageType::NoteOn { note, .. }
            | Midi2MessageType::NoteOff { note, .. }
            | Midi2MessageType::PerNotePitchBend { note, .. }
            | Midi2MessageType::KeyPressure { note, .. }
            | Midi2MessageType::RegisteredPerNoteController { note, .. }
            | Midi2MessageType::AssignablePerNoteController { note, .. }
            | Midi2MessageType::PerNoteManagement { note, .. } => Some(note),
            _ => None,
        }
    }

    /// Get 16-bit velocity (for note on/off events).
    #[inline]
    pub fn velocity_16bit(&self) -> Option<u16> {
        match self.message_type() {
            Midi2MessageType::NoteOn { velocity, .. }
            | Midi2MessageType::NoteOff { velocity, .. } => Some(velocity),
            _ => None,
        }
    }

    /// Get velocity as normalized f32 (0.0-1.0).
    #[inline]
    pub fn velocity_normalized(&self) -> Option<f32> {
        self.velocity_16bit().map(|v| v as f32 / 65535.0)
    }

    /// Get velocity as 7-bit MIDI 1.0 value (downsampled).
    #[inline]
    pub fn velocity(&self) -> Option<u8> {
        self.velocity_16bit().map(midi2_velocity_to_midi1)
    }

    // ==================== Conversion ====================

    /// Convert to MIDI 1.0 event (lossy - reduces resolution).
    ///
    /// Returns None for MIDI 2.0-only message types (per-note pitch bend, per-note controllers).
    pub fn to_midi1(&self) -> Option<crate::MidiEvent> {
        use midi_msg::{Channel, ChannelVoiceMsg, ControlChange};

        use super::convert::midi2_cc_to_midi1;

        let channel = Channel::from_u8(self.channel());
        let frame = self.frame_offset;

        match self.message_type() {
            Midi2MessageType::NoteOn { note, velocity, .. } => {
                let vel = midi2_velocity_to_midi1(velocity);
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::NoteOn {
                        note,
                        velocity: vel,
                    },
                ))
            }
            Midi2MessageType::NoteOff { note, velocity } => {
                let vel = midi2_velocity_to_midi1(velocity);
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::NoteOff {
                        note,
                        velocity: vel,
                    },
                ))
            }
            Midi2MessageType::ControlChange { controller, value } => {
                let val = midi2_cc_to_midi1(value);
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::ControlChange {
                        control: ControlChange::CC {
                            control: controller,
                            value: val,
                        },
                    },
                ))
            }
            Midi2MessageType::ChannelPitchBend { bend } => {
                // MIDI 2.0 pitch bend is 32-bit unsigned, center at 0x80000000
                // MIDI 1.0 pitch bend is 14-bit unsigned, center at 8192
                let bend_14 = (bend >> 18) as u16; // Scale 32-bit to 14-bit
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::PitchBend { bend: bend_14 },
                ))
            }
            Midi2MessageType::ChannelPressure { pressure } => {
                // 32-bit to 7-bit
                let press = (pressure >> 25) as u8;
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::ChannelPressure { pressure: press },
                ))
            }
            Midi2MessageType::KeyPressure { note, pressure } => {
                let press = (pressure >> 25) as u8;
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::PolyPressure {
                        note,
                        pressure: press,
                    },
                ))
            }
            Midi2MessageType::ProgramChange { program, .. } => Some(crate::MidiEvent::new(
                frame,
                channel,
                ChannelVoiceMsg::ProgramChange { program },
            )),
            // MIDI 2.0-only message types have no MIDI 1.0 equivalent
            Midi2MessageType::PerNotePitchBend { .. }
            | Midi2MessageType::RegisteredPerNoteController { .. }
            | Midi2MessageType::AssignablePerNoteController { .. }
            | Midi2MessageType::PerNoteManagement { .. } => None,
            // RPN/NRPN could be converted but would require multiple MIDI 1.0 messages
            Midi2MessageType::RegisteredController { .. }
            | Midi2MessageType::AssignableController { .. } => None,
            Midi2MessageType::Unknown { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi2::convert::midi1_velocity_to_midi2;

    #[test]
    fn test_note_on_creation() {
        let event = Midi2Event::note_on(100, u4::new(0), u4::new(5), u7::new(60), 32768);
        assert_eq!(event.frame_offset, 100);
        assert_eq!(event.channel(), 5);
        assert_eq!(event.group(), 0);
        assert!(event.is_note_on());
        assert!(!event.is_note_off());
        assert_eq!(event.note(), Some(60));
        assert_eq!(event.velocity_16bit(), Some(32768));
    }

    #[test]
    fn test_note_off_creation() {
        let event = Midi2Event::note_off(50, u4::new(1), u4::new(3), u7::new(64), 0);
        assert!(event.is_note_off());
        assert!(!event.is_note_on());
        assert_eq!(event.note(), Some(64));
    }

    #[test]
    fn test_zero_velocity_note_on_is_note_off() {
        let event = Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 0);
        assert!(event.is_note_off());
        assert!(!event.is_note_on());
    }

    #[test]
    fn test_per_note_pitch_bend() {
        let event =
            Midi2Event::per_note_pitch_bend(0, u4::new(0), u4::new(0), u7::new(60), 0x80000000);
        match event.message_type() {
            Midi2MessageType::PerNotePitchBend { note, bend } => {
                assert_eq!(note, 60);
                assert_eq!(bend, 0x80000000);
            }
            _ => panic!("Expected PerNotePitchBend"),
        }
        // Per-note pitch bend has no MIDI 1.0 equivalent
        assert!(event.to_midi1().is_none());
    }

    #[test]
    fn test_control_change() {
        let event = Midi2Event::control_change(0, u4::new(0), u4::new(5), u7::new(7), 0xFFFFFFFF);
        match event.message_type() {
            Midi2MessageType::ControlChange { controller, value } => {
                assert_eq!(controller, 7);
                assert_eq!(value, 0xFFFFFFFF);
            }
            _ => panic!("Expected ControlChange"),
        }
    }

    #[test]
    fn test_midi2_to_midi1_conversion() {
        let midi2 = Midi2Event::note_on(
            100,
            u4::new(0),
            u4::new(5),
            u7::new(60),
            midi1_velocity_to_midi2(100),
        );

        let midi1 = midi2.to_midi1().unwrap();
        assert_eq!(midi1.frame_offset, 100);
        assert_eq!(midi1.channel_num(), 5);
        assert!(midi1.is_note_on());
        assert_eq!(midi1.note(), Some(60));
        assert_eq!(midi1.velocity(), Some(100));
    }

    #[test]
    fn test_normalized_velocity() {
        // Full velocity
        let event = Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 65535);
        let norm = event.velocity_normalized().unwrap();
        assert!((norm - 1.0).abs() < 0.0001);

        // Zero velocity
        let event = Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 0);
        let norm = event.velocity_normalized().unwrap();
        assert!((norm - 0.0).abs() < 0.0001);

        // Half velocity
        let event = Midi2Event::note_on(0, u4::new(0), u4::new(0), u7::new(60), 32768);
        let norm = event.velocity_normalized().unwrap();
        assert!((norm - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_try_from_ump() {
        // Valid channel voice 2 message (type nibble = 0x4)
        let data = [0x40906000, 0x80000000]; // Note On, group 0, ch 0, note 60, vel 0x8000
        let event = Midi2Event::try_from_ump(50, &data).unwrap();
        assert_eq!(event.frame_offset, 50);
        assert!(event.is_note_on());

        // Invalid message type
        let data = [0x20906000, 0x80000000]; // Type = 0x2, not channel voice 2
        assert!(Midi2Event::try_from_ump(0, &data).is_none());

        // Too short
        assert!(Midi2Event::try_from_ump(0, &[0x40906000]).is_none());
    }
}
