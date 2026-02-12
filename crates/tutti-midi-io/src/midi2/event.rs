//! MIDI 2.0 event type with sample-accurate timing.

use midi2::channel_voice2::NoteAttribute;
use midi2::prelude::*;

use super::convert::midi2_velocity_to_midi1;
use super::Midi2MessageType;

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

    /// UMP group (0-15).
    #[inline]
    pub fn group(&self) -> u8 {
        ((self.data[0] >> 24) & 0x0F) as u8
    }

    /// MIDI channel (0-15).
    #[inline]
    pub fn channel(&self) -> u8 {
        ((self.data[0] >> 16) & 0x0F) as u8
    }

    /// Opcode nibble (message type within channel voice 2).
    #[inline]
    fn opcode(&self) -> u8 {
        ((self.data[0] >> 20) & 0x0F) as u8
    }

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
                let bank_valid = (self.data[0] & 0x01) != 0; // Bank valid flag is bit 0
                let bank = if bank_valid {
                    // Bank is stored in octets 2 and 3 of second word as 7-bit values
                    // midi2 crate stores LSB in octet 2, MSB in octet 3
                    let lsb = ((self.data[1] >> 8) & 0x7F) as u8;
                    let msb = (self.data[1] & 0x7F) as u8;
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

    #[inline]
    pub fn is_note_on(&self) -> bool {
        matches!(self.message_type(), Midi2MessageType::NoteOn { velocity, .. } if velocity > 0)
    }

    /// Treats velocity-0 NoteOn as NoteOff per MIDI spec.
    #[inline]
    pub fn is_note_off(&self) -> bool {
        matches!(
            self.message_type(),
            Midi2MessageType::NoteOff { .. } | Midi2MessageType::NoteOn { velocity: 0, .. }
        )
    }

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

    /// 16-bit velocity, or `None` for non-note events.
    #[inline]
    pub fn velocity_16bit(&self) -> Option<u16> {
        match self.message_type() {
            Midi2MessageType::NoteOn { velocity, .. }
            | Midi2MessageType::NoteOff { velocity, .. } => Some(velocity),
            _ => None,
        }
    }

    /// Velocity as normalized f32 (0.0 to 1.0).
    #[inline]
    pub fn velocity_normalized(&self) -> Option<f32> {
        self.velocity_16bit().map(|v| v as f32 / 65535.0)
    }

    /// Velocity downsampled to 7-bit MIDI 1.0 range.
    #[inline]
    pub fn velocity(&self) -> Option<u8> {
        self.velocity_16bit().map(midi2_velocity_to_midi1)
    }

    /// Convert to MIDI 1.0 event (lossy - reduces resolution).
    ///
    /// Returns None for MIDI 2.0-only message types (per-note pitch bend, per-note controllers).
    pub fn to_midi1(&self) -> Option<crate::MidiEvent> {
        use crate::{Channel, ChannelVoiceMsg, ControlChange};

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
                let bend_14 = super::convert::midi2_pitch_bend_to_midi1(bend);
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::PitchBend { bend: bend_14 },
                ))
            }
            Midi2MessageType::ChannelPressure { pressure } => {
                let press = midi2_cc_to_midi1(pressure);
                Some(crate::MidiEvent::new(
                    frame,
                    channel,
                    ChannelVoiceMsg::ChannelPressure { pressure: press },
                ))
            }
            Midi2MessageType::KeyPressure { note, pressure } => {
                let press = midi2_cc_to_midi1(pressure);
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

    #[test]
    fn test_to_midi1_pitch_bend_roundtrip() {
        use crate::midi2::convert::midi1_pitch_bend_to_midi2;

        // Test all 14-bit pitch bend values survive the MIDI 1.0 → 2.0 → to_midi1() roundtrip
        for v in 0..=16383u16 {
            let midi2_event = Midi2Event::channel_pitch_bend(
                0,
                u4::new(0),
                u4::new(0),
                midi1_pitch_bend_to_midi2(v),
            );
            let midi1 = midi2_event.to_midi1().unwrap();
            match midi1.msg {
                crate::ChannelVoiceMsg::PitchBend { bend } => {
                    assert_eq!(
                        bend, v,
                        "to_midi1() pitch bend roundtrip failed for value {}",
                        v
                    );
                }
                _ => panic!("Expected PitchBend"),
            }
        }
    }

    #[test]
    fn test_to_midi1_control_change() {
        use crate::midi2::convert::midi1_cc_to_midi2;

        // CC7 value 100 on channel 3
        let midi2_event = Midi2Event::control_change(
            42,
            u4::new(0),
            u4::new(3),
            u7::new(7),
            midi1_cc_to_midi2(100),
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        assert_eq!(midi1.frame_offset, 42);
        assert_eq!(midi1.channel_num(), 3);
        match midi1.msg {
            crate::ChannelVoiceMsg::ControlChange { control } => match control {
                crate::event::ControlChange::CC { control: cc, value } => {
                    assert_eq!(cc, 7);
                    assert_eq!(value, 100);
                }
                _ => panic!("Expected CC variant"),
            },
            _ => panic!("Expected ControlChange"),
        }

        // Boundary: CC value 0 and 127
        for v in [0u8, 1, 64, 127] {
            let midi2_event = Midi2Event::control_change(
                0,
                u4::new(0),
                u4::new(0),
                u7::new(1),
                midi1_cc_to_midi2(v),
            );
            let midi1 = midi2_event.to_midi1().unwrap();
            match midi1.msg {
                crate::ChannelVoiceMsg::ControlChange { control } => match control {
                    crate::event::ControlChange::CC { value, .. } => {
                        assert_eq!(value, v, "CC roundtrip failed for value {}", v);
                    }
                    _ => panic!("Expected CC variant"),
                },
                _ => panic!("Expected ControlChange"),
            }
        }
    }

    #[test]
    fn test_to_midi1_channel_pressure() {
        // Channel pressure uses >> 25 to convert 32-bit to 7-bit
        // 127 << 25 = 0xFE000000, so max 7-bit maps to near-max 32-bit
        let midi2_event = Midi2Event::channel_pressure(
            10,
            u4::new(0),
            u4::new(5),
            0xFFFFFFFF,
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        assert_eq!(midi1.frame_offset, 10);
        assert_eq!(midi1.channel_num(), 5);
        match midi1.msg {
            crate::ChannelVoiceMsg::ChannelPressure { pressure } => {
                assert_eq!(pressure, 127, "Max 32-bit should map to 127");
            }
            _ => panic!("Expected ChannelPressure"),
        }

        // Zero pressure
        let midi2_event = Midi2Event::channel_pressure(
            0,
            u4::new(0),
            u4::new(0),
            0,
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        match midi1.msg {
            crate::ChannelVoiceMsg::ChannelPressure { pressure } => {
                assert_eq!(pressure, 0);
            }
            _ => panic!("Expected ChannelPressure"),
        }
    }

    #[test]
    fn test_to_midi1_key_pressure() {
        // Per-note pressure (poly aftertouch)
        let midi2_event = Midi2Event::key_pressure(
            20,
            u4::new(0),
            u4::new(2),
            u7::new(72),
            0xFFFFFFFF,
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        assert_eq!(midi1.frame_offset, 20);
        assert_eq!(midi1.channel_num(), 2);
        match midi1.msg {
            crate::ChannelVoiceMsg::PolyPressure { note, pressure } => {
                assert_eq!(note, 72);
                assert_eq!(pressure, 127);
            }
            _ => panic!("Expected PolyPressure"),
        }

        // Mid-value
        let mid_32 = 0x80000000u32; // ~64 in 7-bit
        let midi2_event = Midi2Event::key_pressure(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            mid_32,
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        match midi1.msg {
            crate::ChannelVoiceMsg::PolyPressure { note, pressure } => {
                assert_eq!(note, 60);
                assert_eq!(pressure, 64, "0x80000000 should map to 64");
            }
            _ => panic!("Expected PolyPressure"),
        }
    }

    #[test]
    fn test_to_midi1_pressure_roundtrip() {
        use crate::midi2::convert::midi1_cc_to_midi2;

        // All 7-bit pressure values should survive MIDI 1.0 → 2.0 → to_midi1() roundtrip
        for v in 0..=127u8 {
            let midi2_val = midi1_cc_to_midi2(v);

            // Channel pressure
            let event = Midi2Event::channel_pressure(0, u4::new(0), u4::new(0), midi2_val);
            let midi1 = event.to_midi1().unwrap();
            match midi1.msg {
                crate::ChannelVoiceMsg::ChannelPressure { pressure } => {
                    assert_eq!(pressure, v, "Channel pressure roundtrip failed for {}", v);
                }
                _ => panic!("Expected ChannelPressure"),
            }

            // Key pressure
            let event = Midi2Event::key_pressure(0, u4::new(0), u4::new(0), u7::new(60), midi2_val);
            let midi1 = event.to_midi1().unwrap();
            match midi1.msg {
                crate::ChannelVoiceMsg::PolyPressure { pressure, .. } => {
                    assert_eq!(pressure, v, "Key pressure roundtrip failed for {}", v);
                }
                _ => panic!("Expected PolyPressure"),
            }
        }
    }

    #[test]
    fn test_to_midi1_program_change() {
        // Program change without bank select
        let midi2_event = Midi2Event::program_change(
            5,
            u4::new(0),
            u4::new(9),
            u7::new(42),
            None,
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        assert_eq!(midi1.frame_offset, 5);
        assert_eq!(midi1.channel_num(), 9);
        match midi1.msg {
            crate::ChannelVoiceMsg::ProgramChange { program } => {
                assert_eq!(program, 42);
            }
            _ => panic!("Expected ProgramChange"),
        }

        // Program change with bank select (bank is dropped in MIDI 1.0 conversion)
        let midi2_event = Midi2Event::program_change(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(0),
            Some(u14::new(128)),
        );
        let midi1 = midi2_event.to_midi1().unwrap();
        match midi1.msg {
            crate::ChannelVoiceMsg::ProgramChange { program } => {
                assert_eq!(program, 0);
            }
            _ => panic!("Expected ProgramChange"),
        }
    }

    #[test]
    fn test_program_change_bank_roundtrip() {
        // Create a ProgramChange with bank select
        let event = Midi2Event::program_change(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(42),
            Some(u14::new(128)),
        );

        // Verify bank_valid flag is detected
        match event.message_type() {
            Midi2MessageType::ProgramChange { program, bank } => {
                assert_eq!(program, 42);
                assert!(bank.is_some(), "Bank should be Some when set via program_change constructor");
                let bank_val = bank.unwrap();
                assert_eq!(bank_val, 128, "Bank value should roundtrip correctly, got {}", bank_val);
            }
            _ => panic!("Expected ProgramChange"),
        }

        // Without bank
        let event = Midi2Event::program_change(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(0),
            None,
        );
        match event.message_type() {
            Midi2MessageType::ProgramChange { bank, .. } => {
                assert!(bank.is_none(), "Bank should be None when not set");
            }
            _ => panic!("Expected ProgramChange"),
        }
    }

    #[test]
    fn test_to_midi1_returns_none_for_midi2_only_types() {
        // Per-note pitch bend
        let event = Midi2Event::per_note_pitch_bend(0, u4::new(0), u4::new(0), u7::new(60), 0x80000000);
        assert!(event.to_midi1().is_none());

        // Registered per-note controller (raw UMP)
        let event = Midi2Event::try_from_ump(0, &[0x4000_3C4A, 0xFFFFFFFF]).unwrap();
        assert!(event.to_midi1().is_none());

        // Assignable per-note controller (raw UMP)
        let event = Midi2Event::try_from_ump(0, &[0x4010_3C4A, 0xFFFFFFFF]).unwrap();
        assert!(event.to_midi1().is_none());

        // RPN (raw UMP: opcode 0x2)
        let event = Midi2Event::try_from_ump(0, &[0x4020_0100, 0x12345678]).unwrap();
        assert!(event.to_midi1().is_none());

        // NRPN (raw UMP: opcode 0x3)
        let event = Midi2Event::try_from_ump(0, &[0x4030_0100, 0x12345678]).unwrap();
        assert!(event.to_midi1().is_none());
    }

    #[test]
    fn test_note_on_with_attribute() {
        let event = Midi2Event::note_on_with_attribute(
            100,
            u4::new(0),
            u4::new(5),
            u7::new(60),
            32768,
            NoteAttribute::ManufacturerSpecific(0x1234),
        );
        assert_eq!(event.frame_offset, 100);
        assert_eq!(event.channel(), 5);
        assert!(event.is_note_on());
        assert_eq!(event.note(), Some(60));
        assert_eq!(event.velocity_16bit(), Some(32768));

        // Verify attribute is present
        match event.message_type() {
            Midi2MessageType::NoteOn { attribute, .. } => {
                assert!(attribute.is_some(), "Attribute should be present");
            }
            _ => panic!("Expected NoteOn"),
        }

        // Without attribute — attr_type byte should be 0
        let event_no_attr = Midi2Event::note_on(
            0,
            u4::new(0),
            u4::new(0),
            u7::new(60),
            32768,
        );
        match event_no_attr.message_type() {
            Midi2MessageType::NoteOn { attribute, .. } => {
                assert!(attribute.is_none(), "No attribute should be present");
            }
            _ => panic!("Expected NoteOn"),
        }
    }

    #[test]
    fn test_message_type_per_note_management() {
        // Per-note management: opcode 0xF, detach + reset flags
        // UMP format: 0x40F0_NNDD where NN=note, DD=flags (bit 1=detach, bit 0=reset)
        let event = Midi2Event::try_from_ump(0, &[0x40F0_3C03, 0x00000000]).unwrap();
        match event.message_type() {
            Midi2MessageType::PerNoteManagement { note, detach, reset } => {
                assert_eq!(note, 60);
                assert!(detach);
                assert!(reset);
            }
            _ => panic!("Expected PerNoteManagement"),
        }

        // Note is extracted from to_midi1() as None (MIDI 2.0 only)
        assert!(event.to_midi1().is_none());
    }

    #[test]
    fn test_message_type_unknown_opcode() {
        // Opcode 0x7 is not defined → Unknown
        let event = Midi2Event::try_from_ump(0, &[0x4070_0000, 0x00000000]).unwrap();
        match event.message_type() {
            Midi2MessageType::Unknown { opcode } => {
                assert_eq!(opcode, 0x7);
            }
            _ => panic!("Expected Unknown"),
        }
        assert!(event.to_midi1().is_none());
    }
}
