//! Parsed MIDI 2.0 message types.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Midi2MessageType {
    NoteOn {
        note: u8,
        velocity: u16,
        attribute: Option<u16>,
    },
    NoteOff {
        note: u8,
        velocity: u16,
    },
    /// Per-note pitch bend (32-bit, center at 0x80000000)
    PerNotePitchBend {
        note: u8,
        bend: u32,
    },
    ControlChange {
        controller: u8,
        value: u32,
    },
    ChannelPitchBend {
        bend: u32,
    },
    ChannelPressure {
        pressure: u32,
    },
    KeyPressure {
        note: u8,
        pressure: u32,
    },
    /// Program change with optional 14-bit bank select
    ProgramChange {
        program: u8,
        /// Bank as 14-bit value (MSB << 7 | LSB)
        bank: Option<u16>,
    },
    RegisteredPerNoteController {
        note: u8,
        index: u8,
        value: u32,
    },
    AssignablePerNoteController {
        note: u8,
        index: u8,
        value: u32,
    },
    RegisteredController {
        bank: u8,
        index: u8,
        value: u32,
    },
    AssignableController {
        bank: u8,
        index: u8,
        value: u32,
    },
    PerNoteManagement {
        note: u8,
        detach: bool,
        reset: bool,
    },
    Unknown {
        opcode: u8,
    },
}
