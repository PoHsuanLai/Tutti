//! Fluent MIDI output builder

use crate::system::MidiSystem;

/// Fluent builder for sending MIDI messages. Created via `MidiSystem::send()`.
pub struct MidiBuilder<'a> {
    #[cfg(feature = "midi-io")]
    midi: Option<&'a MidiSystem>,
    #[cfg(not(feature = "midi-io"))]
    _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a> MidiBuilder<'a> {
    #[cfg(feature = "midi-io")]
    pub(crate) fn new(midi: Option<&'a MidiSystem>) -> Self {
        Self { midi }
    }

    #[cfg(not(feature = "midi-io"))]
    pub(crate) fn new(_midi: Option<&'a MidiSystem>) -> Self {
        Self {
            _phantom: std::marker::PhantomData,
        }
    }

    /// Channel 0-15, note 0-127, velocity 0-127.
    #[cfg(feature = "midi-io")]
    pub fn note_on(self, channel: u8, note: u8, velocity: u8) -> Self {
        if let Some(midi) = self.midi {
            let _ = midi.send_note_on(channel, note, velocity);
        }
        self
    }

    #[cfg(not(feature = "midi-io"))]
    pub fn note_on(self, _channel: u8, _note: u8, _velocity: u8) -> Self {
        self
    }

    /// Channel 0-15, note 0-127, release velocity 0-127.
    #[cfg(feature = "midi-io")]
    pub fn note_off(self, channel: u8, note: u8, velocity: u8) -> Self {
        if let Some(midi) = self.midi {
            let _ = midi.send_note_off(channel, note, velocity);
        }
        self
    }

    #[cfg(not(feature = "midi-io"))]
    pub fn note_off(self, _channel: u8, _note: u8, _velocity: u8) -> Self {
        self
    }

    /// Channel 0-15, controller 0-127, value 0-127.
    #[cfg(feature = "midi-io")]
    pub fn cc(self, channel: u8, cc: u8, value: u8) -> Self {
        if let Some(midi) = self.midi {
            let _ = midi.send_cc(channel, cc, value);
        }
        self
    }

    #[cfg(not(feature = "midi-io"))]
    pub fn cc(self, _channel: u8, _cc: u8, _value: u8) -> Self {
        self
    }

    /// Channel 0-15, value -8192..=8191 (0 = center).
    #[cfg(feature = "midi-io")]
    pub fn pitch_bend(self, channel: u8, value: i16) -> Self {
        if let Some(midi) = self.midi {
            let _ = midi.send_pitch_bend(channel, value);
        }
        self
    }

    #[cfg(not(feature = "midi-io"))]
    pub fn pitch_bend(self, _channel: u8, _value: i16) -> Self {
        self
    }

    /// Channel 0-15, program 0-127.
    #[cfg(feature = "midi-io")]
    pub fn program_change(self, channel: u8, program: u8) -> Self {
        if let Some(midi) = self.midi {
            let _ = midi.send_program_change(channel, program);
        }
        self
    }

    #[cfg(not(feature = "midi-io"))]
    pub fn program_change(self, _channel: u8, _program: u8) -> Self {
        self
    }
}
