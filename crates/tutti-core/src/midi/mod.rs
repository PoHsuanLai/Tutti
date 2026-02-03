//! MIDI support for Tutti audio graph.
//!
//! This module provides MIDI types, traits and routing infrastructure:
//! - [`event`]: Core MIDI event types (`MidiEvent`, `RawMidiEvent`)
//! - [`traits`]: MIDI-aware AudioUnit traits
//! - [`registry`]: RT-safe MIDI event routing system

pub mod event;
pub mod registry;
pub mod traits;

// Re-export commonly used types
pub use event::{MidiEvent, MidiEventBuilder, RawMidiEvent};
pub use registry::MidiRegistry;
pub use traits::{AsMidiAudioUnit, MidiAudioUnit};

// Re-export midi-msg types that users need
pub use midi_msg::{Channel, ChannelVoiceMsg, ControlChange, MidiMsg};
