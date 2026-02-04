//! Parsed MIDI 2.0 message types.

/// Parsed MIDI 2.0 message type with extracted data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Midi2MessageType {
    /// Note On with 16-bit velocity and optional attribute
    NoteOn {
        note: u8,
        velocity: u16,
        attribute: Option<u16>,
    },
    /// Note Off with 16-bit velocity
    NoteOff { note: u8, velocity: u16 },
    /// Per-note pitch bend (32-bit, center at 0x80000000)
    PerNotePitchBend { note: u8, bend: u32 },
    /// Control Change (32-bit value)
    ControlChange { controller: u8, value: u32 },
    /// Channel-wide pitch bend (32-bit, center at 0x80000000)
    ChannelPitchBend { bend: u32 },
    /// Channel pressure/aftertouch (32-bit)
    ChannelPressure { pressure: u32 },
    /// Per-note pressure/aftertouch (32-bit)
    KeyPressure { note: u8, pressure: u32 },
    /// Program change with optional 14-bit bank select
    ProgramChange {
        program: u8,
        /// Bank as 14-bit value (MSB << 7 | LSB)
        bank: Option<u16>,
    },
    /// Registered per-note controller
    RegisteredPerNoteController { note: u8, index: u8, value: u32 },
    /// Assignable per-note controller
    AssignablePerNoteController { note: u8, index: u8, value: u32 },
    /// Registered controller (RPN)
    RegisteredController { bank: u8, index: u8, value: u32 },
    /// Assignable controller (NRPN)
    AssignableController { bank: u8, index: u8, value: u32 },
    /// Per-note management (detach, reset)
    PerNoteManagement { note: u8, detach: bool, reset: bool },
    /// Unknown opcode
    Unknown { opcode: u8 },
}
