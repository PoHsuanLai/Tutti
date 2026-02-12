//! Pure MIDI types for the Tutti audio engine.
//!
//! This crate provides `no_std`-compatible MIDI types used throughout the Tutti
//! ecosystem. It is the single source of truth for core MIDI data structures.
//!
//! Higher-level MIDI infrastructure (routing, registry, traits) lives in `tutti-core`.
//! Hardware I/O, MPE, MIDI 2.0, and CC mapping live in `tutti-midi-io`.

#![no_std]

#[cfg(feature = "std")]
extern crate std;

extern crate alloc;

pub(crate) mod compat;

pub mod event;
pub mod input_source;
pub mod note;
pub mod utils;

// Re-export commonly used types
pub use event::{MidiEvent, MidiEventBuilder, RawMidiEvent};
pub use input_source::{MidiInputSource, NoMidiInput};
pub use note::Note;
pub use utils::{gain_to_velocity, hz_to_note, note_to_hz, velocity_to_gain};

// Re-export midi-msg types that users need
pub use midi_msg::{
    Channel, ChannelModeMsg, ChannelVoiceMsg, ControlChange, MidiMsg, SystemCommonMsg,
    SystemRealTimeMsg,
};
