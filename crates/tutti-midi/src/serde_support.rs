//! Serde support for MIDI events
//!
//! This module provides `Serialize` and `Deserialize` implementations for
//! `MidiEvent` to enable IPC communication in the plugin hosting system.

use crate::event::MidiEvent;
use midi_msg::{Channel, ChannelVoiceMsg, ControlChange};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// Serializable representation of MidiEvent
#[derive(Serialize, Deserialize)]
struct SerializableMidiEvent {
    frame_offset: usize,
    channel: u8,
    msg_type: u8,
    data: MsgData,
}

#[derive(Serialize, Deserialize)]
enum MsgData {
    NoteOn { note: u8, velocity: u8 },
    NoteOff { note: u8, velocity: u8 },
    HighResNoteOn { note: u8, velocity: u16 },
    HighResNoteOff { note: u8, velocity: u16 },
    PolyPressure { note: u8, pressure: u8 },
    ControlChange { control: u8, value: u8 },
    ProgramChange { program: u8 },
    ChannelPressure { pressure: u8 },
    PitchBend { bend: u16 },
}

impl Serialize for MidiEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let msg_data = match self.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => MsgData::NoteOn { note, velocity },
            ChannelVoiceMsg::NoteOff { note, velocity } => MsgData::NoteOff { note, velocity },
            ChannelVoiceMsg::HighResNoteOn { note, velocity } => {
                MsgData::HighResNoteOn { note, velocity }
            }
            ChannelVoiceMsg::HighResNoteOff { note, velocity } => {
                MsgData::HighResNoteOff { note, velocity }
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                MsgData::PolyPressure { note, pressure }
            }
            ChannelVoiceMsg::ControlChange { control } => {
                // Extract control number and value
                let (cc, value) = match control {
                    ControlChange::CC { control, value } => (control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => {
                        // Convert 14-bit to 7-bit for serialization
                        (control1, (value >> 7) as u8)
                    }
                    // For other CC types, use default values
                    _ => (0, 0),
                };
                MsgData::ControlChange { control: cc, value }
            }
            ChannelVoiceMsg::ProgramChange { program } => MsgData::ProgramChange { program },
            ChannelVoiceMsg::ChannelPressure { pressure } => MsgData::ChannelPressure { pressure },
            ChannelVoiceMsg::PitchBend { bend } => MsgData::PitchBend { bend },
        };

        let serializable = SerializableMidiEvent {
            frame_offset: self.frame_offset,
            channel: self.channel as u8,
            msg_type: 0, // Not used, but kept for backwards compat
            data: msg_data,
        };

        serializable.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for MidiEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serializable = SerializableMidiEvent::deserialize(deserializer)?;

        let msg = match serializable.data {
            MsgData::NoteOn { note, velocity } => ChannelVoiceMsg::NoteOn { note, velocity },
            MsgData::NoteOff { note, velocity } => ChannelVoiceMsg::NoteOff { note, velocity },
            MsgData::HighResNoteOn { note, velocity } => {
                ChannelVoiceMsg::HighResNoteOn { note, velocity }
            }
            MsgData::HighResNoteOff { note, velocity } => {
                ChannelVoiceMsg::HighResNoteOff { note, velocity }
            }
            MsgData::PolyPressure { note, pressure } => {
                ChannelVoiceMsg::PolyPressure { note, pressure }
            }
            MsgData::ControlChange { control, value } => ChannelVoiceMsg::ControlChange {
                control: ControlChange::CC { control, value },
            },
            MsgData::ProgramChange { program } => ChannelVoiceMsg::ProgramChange { program },
            MsgData::ChannelPressure { pressure } => ChannelVoiceMsg::ChannelPressure { pressure },
            MsgData::PitchBend { bend } => ChannelVoiceMsg::PitchBend { bend },
        };

        Ok(MidiEvent {
            frame_offset: serializable.frame_offset,
            channel: Channel::from_u8(serializable.channel),
            msg,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_on_serialization() {
        let event = MidiEvent::note_on(128, 0, 60, 100);
        let serialized = bincode::serialize(&event).unwrap();
        let deserialized: MidiEvent = bincode::deserialize(&serialized).unwrap();

        assert_eq!(event.frame_offset, deserialized.frame_offset);
        assert_eq!(event.channel, deserialized.channel);
        assert_eq!(event.msg, deserialized.msg);
    }

    #[test]
    fn test_note_off_serialization() {
        let event = MidiEvent::note_off(256, 1, 72, 64);
        let serialized = bincode::serialize(&event).unwrap();
        let deserialized: MidiEvent = bincode::deserialize(&serialized).unwrap();

        assert_eq!(event.frame_offset, deserialized.frame_offset);
        assert_eq!(event.channel, deserialized.channel);
        assert_eq!(event.msg, deserialized.msg);
    }

    #[test]
    fn test_control_change_serialization() {
        let event = MidiEvent::control_change(0, 0, 7, 127);
        let serialized = bincode::serialize(&event).unwrap();
        let deserialized: MidiEvent = bincode::deserialize(&serialized).unwrap();

        assert_eq!(event.frame_offset, deserialized.frame_offset);
        assert_eq!(event.channel, deserialized.channel);
        assert_eq!(event.msg, deserialized.msg);
    }

    #[test]
    fn test_pitch_bend_serialization() {
        let event = MidiEvent::pitch_bend(100, 2, 8192);
        let serialized = bincode::serialize(&event).unwrap();
        let deserialized: MidiEvent = bincode::deserialize(&serialized).unwrap();

        assert_eq!(event.frame_offset, deserialized.frame_offset);
        assert_eq!(event.channel, deserialized.channel);
        assert_eq!(event.msg, deserialized.msg);
    }

    #[test]
    fn test_program_change_serialization() {
        let event = MidiEvent::program_change(0, 3, 42);
        let serialized = bincode::serialize(&event).unwrap();
        let deserialized: MidiEvent = bincode::deserialize(&serialized).unwrap();

        assert_eq!(event.frame_offset, deserialized.frame_offset);
        assert_eq!(event.channel, deserialized.channel);
        assert_eq!(event.msg, deserialized.msg);
    }

    #[test]
    fn test_roundtrip_multiple_events() {
        let events = vec![
            MidiEvent::note_on(0, 0, 60, 100),
            MidiEvent::note_on(128, 0, 64, 100),
            MidiEvent::control_change(256, 0, 7, 64),
            MidiEvent::note_off(512, 0, 60, 0),
            MidiEvent::note_off(640, 0, 64, 0),
        ];

        let serialized = bincode::serialize(&events).unwrap();
        let deserialized: Vec<MidiEvent> = bincode::deserialize(&serialized).unwrap();

        assert_eq!(events.len(), deserialized.len());
        for (orig, deser) in events.iter().zip(deserialized.iter()) {
            assert_eq!(orig.frame_offset, deser.frame_offset);
            assert_eq!(orig.channel, deser.channel);
            assert_eq!(orig.msg, deser.msg);
        }
    }
}
