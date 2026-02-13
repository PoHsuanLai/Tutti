//! RT-safe MIDI event types with sample-accurate timing.

use midi_msg::{Channel, ChannelVoiceMsg, MidiMsg};

use crate::compat::Vec;

/// RT-safe MIDI event with sample-accurate frame offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiEvent {
    /// Offset within the current buffer (0 = first sample).
    pub frame_offset: usize,
    pub channel: Channel,
    pub msg: ChannelVoiceMsg,
}

impl MidiEvent {
    #[inline]
    pub fn new(frame_offset: usize, channel: Channel, msg: ChannelVoiceMsg) -> Self {
        Self {
            frame_offset,
            channel,
            msg,
        }
    }

    #[inline]
    pub fn note_on_builder(note: u8, velocity: u8) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::NoteOn { note, velocity },
        }
    }

    #[inline]
    pub fn note_off_builder(note: u8) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::NoteOff { note, velocity: 0 },
        }
    }

    #[inline]
    pub fn cc_builder(control: u8, value: u8) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::ControlChange {
                control: midi_msg::ControlChange::CC { control, value },
            },
        }
    }

    #[inline]
    pub fn bend_builder(bend: u16) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::PitchBend { bend },
        }
    }

    #[inline]
    pub fn program_builder(program: u8) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::ProgramChange { program },
        }
    }

    #[inline]
    pub fn aftertouch_builder(pressure: u8) -> MidiEventBuilder {
        MidiEventBuilder {
            frame_offset: 0,
            channel: 0,
            msg: ChannelVoiceMsg::ChannelPressure { pressure },
        }
    }

    #[inline]
    pub fn note_on(frame_offset: usize, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::NoteOn { note, velocity },
        }
    }

    #[inline]
    pub fn note_off(frame_offset: usize, channel: u8, note: u8, velocity: u8) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::NoteOff { note, velocity },
        }
    }

    #[inline]
    pub fn control_change(frame_offset: usize, channel: u8, cc: u8, value: u8) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::ControlChange {
                control: midi_msg::ControlChange::CC { control: cc, value },
            },
        }
    }

    #[inline]
    pub fn pitch_bend(frame_offset: usize, channel: u8, bend: u16) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::PitchBend { bend },
        }
    }

    #[inline]
    pub fn aftertouch(frame_offset: usize, channel: u8, pressure: u8) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::ChannelPressure { pressure },
        }
    }

    #[inline]
    pub fn poly_aftertouch(frame_offset: usize, channel: u8, note: u8, pressure: u8) -> Self {
        Self {
            frame_offset,
            channel: Channel::from_u8(channel),
            msg: ChannelVoiceMsg::PolyPressure { note, pressure },
        }
    }

    #[inline]
    pub fn channel_num(&self) -> u8 {
        self.channel as u8
    }

    #[inline]
    pub fn is_note_on(&self) -> bool {
        matches!(self.msg, ChannelVoiceMsg::NoteOn { velocity, .. } if velocity > 0)
    }

    #[inline]
    pub fn is_note_off(&self) -> bool {
        matches!(
            self.msg,
            ChannelVoiceMsg::NoteOff { .. } | ChannelVoiceMsg::NoteOn { velocity: 0, .. }
        )
    }

    #[inline]
    pub fn note(&self) -> Option<u8> {
        match self.msg {
            ChannelVoiceMsg::NoteOn { note, .. }
            | ChannelVoiceMsg::NoteOff { note, .. }
            | ChannelVoiceMsg::HighResNoteOn { note, .. }
            | ChannelVoiceMsg::HighResNoteOff { note, .. }
            | ChannelVoiceMsg::PolyPressure { note, .. } => Some(note),
            _ => None,
        }
    }

    #[inline]
    pub fn velocity(&self) -> Option<u8> {
        match self.msg {
            ChannelVoiceMsg::NoteOn { velocity, .. }
            | ChannelVoiceMsg::NoteOff { velocity, .. } => Some(velocity),
            ChannelVoiceMsg::HighResNoteOn { velocity, .. }
            | ChannelVoiceMsg::HighResNoteOff { velocity, .. } => {
                // High-res velocity is 14-bit, return upper 7 bits
                Some((velocity >> 7) as u8)
            }
            _ => None,
        }
    }

    #[inline]
    pub fn to_midi_msg(&self) -> MidiMsg {
        MidiMsg::ChannelVoice {
            channel: self.channel,
            msg: self.msg,
        }
    }

    #[inline]
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_midi_msg().to_midi()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, midi_msg::ParseError> {
        Self::from_bytes_with_offset(bytes, 0)
    }

    pub fn from_bytes_with_offset(
        bytes: &[u8],
        frame_offset: usize,
    ) -> Result<Self, midi_msg::ParseError> {
        let (msg, _len) = MidiMsg::from_midi(bytes)?;
        match msg {
            MidiMsg::ChannelVoice { channel, msg } => Ok(Self {
                frame_offset,
                channel,
                msg,
            }),
            _ => Err(midi_msg::ParseError::Invalid(
                "Expected ChannelVoice message",
            )),
        }
    }
}

/// Raw 3-byte MIDI event for unparsed storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawMidiEvent {
    pub frame_offset: usize,
    pub data: [u8; 3],
    /// Valid bytes in `data` (1-3).
    pub len: u8,
}

impl RawMidiEvent {
    #[inline]
    pub fn new(frame_offset: usize, data: [u8; 3], len: u8) -> Self {
        Self {
            frame_offset,
            data,
            len,
        }
    }

    #[inline]
    pub fn status(&self) -> u8 {
        self.data[0] & 0xF0
    }

    #[inline]
    pub fn channel(&self) -> u8 {
        self.data[0] & 0x0F
    }

    pub fn to_midi_event(&self) -> Result<MidiEvent, midi_msg::ParseError> {
        MidiEvent::from_bytes_with_offset(&self.data[..self.len as usize], self.frame_offset)
    }
}

impl From<MidiEvent> for RawMidiEvent {
    fn from(event: MidiEvent) -> Self {
        let bytes = event.to_bytes();
        let mut data = [0u8; 3];
        let len = bytes.len().min(3);
        data[..len].copy_from_slice(&bytes[..len]);
        Self {
            frame_offset: event.frame_offset,
            data,
            len: len as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MidiEventBuilder {
    frame_offset: usize,
    channel: u8,
    msg: ChannelVoiceMsg,
}

impl MidiEventBuilder {
    #[inline]
    pub fn channel(mut self, channel: u8) -> Self {
        self.channel = channel;
        self
    }

    #[inline]
    pub fn offset(mut self, frame_offset: usize) -> Self {
        self.frame_offset = frame_offset;
        self
    }

    #[inline]
    pub fn build(self) -> MidiEvent {
        MidiEvent {
            frame_offset: self.frame_offset,
            channel: Channel::from_u8(self.channel),
            msg: self.msg,
        }
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
                midi_msg::ControlChange::CC { control: cc, value } => {
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
        let event = MidiEvent::pitch_bend(0, 0, 8192);
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
    }

    #[test]
    fn test_builder_with_offset() {
        let event = MidiEvent::note_on_builder(67, 120).offset(480).build();
        assert_eq!(event.frame_offset, 480);
    }

    #[test]
    fn test_builder_cc() {
        let event = MidiEvent::cc_builder(7, 127).channel(2).build();
        assert_eq!(event.channel_num(), 2);
    }
}
