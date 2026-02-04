//! MIDI support for Tutti audio graph.
//!
//! This module provides MIDI types, traits and routing infrastructure:
//! - [`event`]: Core MIDI event types (`MidiEvent`, `RawMidiEvent`)
//! - [`traits`]: MIDI-aware AudioUnit traits
//! - [`registry`]: RT-safe MIDI event routing system
//! - [`input_source`]: Abstraction for MIDI input sources (hardware, virtual ports)
//! - [`routing`]: Lock-free MIDI routing table for channel/port/layer routing

pub mod event;
pub mod input_source;
pub mod registry;
pub mod routing;
pub mod traits;

// Re-export commonly used types
pub use event::{MidiEvent, MidiEventBuilder, RawMidiEvent};
pub use input_source::{MidiInputSource, NoMidiInput};
pub use registry::MidiRegistry;
pub use routing::{MidiRoute, MidiRoutingSnapshot, MidiRoutingTable};
pub use traits::{AsMidiAudioUnit, MidiAudioUnit};

// Re-export midi-msg types that users need
pub use midi_msg::{Channel, ChannelVoiceMsg, ControlChange, MidiMsg};
