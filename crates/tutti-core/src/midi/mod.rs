//! MIDI support for Tutti audio graph.
//!
//! Pure MIDI types (`MidiEvent`, `RawMidiEvent`, `MidiInputSource`, etc.) live in
//! the `tutti-midi` crate and are re-exported here for convenience.
//!
//! This module adds higher-level infrastructure that depends on tutti-core internals:
//! - [`registry`]: RT-safe MIDI event routing system (depends on crossbeam/dashmap)
//! - [`routing`]: Lock-free MIDI routing table (depends on arc_swap)

pub mod registry;
pub mod routing;

// Re-export pure MIDI types from tutti-midi
pub use tutti_midi::{
    event, input_source, Channel, ChannelModeMsg, ChannelVoiceMsg, ControlChange, MidiEvent,
    MidiEventBuilder, MidiInputSource, MidiMsg, NoMidiInput, RawMidiEvent, SystemCommonMsg,
    SystemRealTimeMsg,
};

// Re-export local higher-level types
pub use registry::MidiRegistry;
pub use routing::{MidiRoute, MidiRoutingSnapshot, MidiRoutingTable};
