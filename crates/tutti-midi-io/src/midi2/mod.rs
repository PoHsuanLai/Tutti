//! MIDI 2.0 high-resolution messages.
//!
//! This module provides MIDI 2.0 (UMP) support:
//! - [`Midi2Event`]: RT-safe MIDI 2.0 event with sample-accurate timing
//! - [`Midi2MessageType`]: Parsed message type enum
//! - Conversion functions between MIDI 1.0 and MIDI 2.0

mod convert;
mod event;
mod message_type;

pub use convert::{
    midi1_cc_to_midi2, midi1_pitch_bend_to_midi2, midi1_to_midi2, midi1_velocity_to_midi2,
    midi2_cc_to_midi1, midi2_pitch_bend_to_midi1, midi2_velocity_to_midi1,
};
pub use event::Midi2Event;
pub use message_type::Midi2MessageType;
