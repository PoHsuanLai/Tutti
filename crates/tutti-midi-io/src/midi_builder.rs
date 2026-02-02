//! Fluent MIDI output builder

use crate::system::MidiSystem;

/// Fluent builder for sending MIDI messages.
///
/// Created via `MidiSystem::send()`.
///
/// # Example
/// ```ignore
/// // Chain multiple messages
/// midi.send()
///     .note_on(0, 60, 100)
///     .cc(0, 74, 64)
///     .pitch_bend(0, 0);
///
/// // Or single message
/// midi.send().note_on(0, 60, 100);
/// ```
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

    /// Send a Note On message.
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `note` - Note number (0-127)
    /// * `velocity` - Velocity (0-127)
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

    /// Send a Note Off message.
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `note` - Note number (0-127)
    /// * `velocity` - Release velocity (0-127)
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

    /// Send a Control Change (CC) message.
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `cc` - Controller number (0-127)
    /// * `value` - Controller value (0-127)
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

    /// Send a Pitch Bend message.
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `value` - Pitch bend value (-8192 to 8191, 0 = center)
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

    /// Send Program Change message.
    ///
    /// # Arguments
    /// * `channel` - MIDI channel (0-15)
    /// * `program` - Program number (0-127)
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
