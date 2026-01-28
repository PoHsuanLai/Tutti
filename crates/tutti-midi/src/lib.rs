//! MIDI subsystem for Tutti audio engine.
//!
//! Provides port management, hardware I/O, MPE, MIDI 2.0, CC mapping, and output collection.
//!
//! # Features
//!
//! - **Port management**: Virtual MIDI ports for routing
//! - **Hardware I/O**: Device enumeration and real-time I/O (feature: `midi-io`)
//! - **MPE**: MIDI Polyphonic Expression (feature: `mpe`)
//! - **MIDI 2.0**: High-resolution messages (feature: `midi2`)
//! - **CC mapping**: MIDI learn and parameter control
//! - **Output collection**: Lock-free MIDI output from audio nodes
//!
//! # Example
//!
//! ```ignore
//! use tutti_midi::MidiSystem;
//!
//! // Basic MIDI I/O
//! let midi = MidiSystem::builder()
//!     .io()
//!     .build()?;
//! midi.connect_device_by_name("Keyboard")?;
//! midi.send_note_on(0, 60, 100)?;
//!
//! // With CC mapping
//! let midi = MidiSystem::builder()
//!     .io()
//!     .cc_mapping()
//!     .build()?;
//!
//! if let Some(cc_mgr) = midi.cc_manager() {
//!     cc_mgr.add_mapping(Some(0), 74, CCTarget::MasterVolume, 0.0, 1.0);
//! }
//! ```

// Error types
pub mod error;
pub use error::{Error, Result};

// Main entry point - MidiSystem
mod system;
pub use system::{MidiSystem, MidiSystemBuilder};

// Sub-handles
#[cfg(feature = "midi2")]
pub use system::Midi2Handle;
#[cfg(feature = "mpe")]
pub use system::MpeHandle;

// Essential types users need
pub use event::{MidiEvent, RawMidiEvent};
pub use multi_port::PortInfo;

// MPE configuration types
#[cfg(feature = "mpe")]
pub use mpe::{MpeMode, MpeZone, MpeZoneConfig};

// MIDI 2.0 types
#[cfg(feature = "midi2")]
pub use event::UnifiedMidiEvent;
#[cfg(feature = "midi2")]
pub use midi2::{Midi2Event, Midi2MessageType};

// Hardware device info
#[cfg(feature = "midi-io")]
pub use input::MidiInputDevice;

// MIDI file types
pub use file::{MidiEventType, ParsedMidiFile, TimedMidiEvent};

// Utility functions
pub use utils::{gain_to_velocity, hz_to_note, note_to_hz, velocity_to_gain};

// CC mapping types
pub use cc_manager::{CCMappingManager, CCProcessResult};
pub use cc_mapping::{CCMapping, CCMappingRegistry, CCNumber, CCTarget, MappingId, MidiChannel};

// MIDI output collection types
pub use output_collector::{
    midi_output_channel, midi_output_channel_with_capacity, MidiOutputAggregator,
    MidiOutputConsumer, MidiOutputProducer,
};

// Re-export essential upstream types (users shouldn't need to import midi-msg directly)
pub use midi_msg::{
    Channel, ChannelModeMsg, ChannelVoiceMsg, ControlChange, MidiMsg, SystemCommonMsg,
    SystemRealTimeMsg,
};

pub(crate) mod async_port;
pub(crate) mod event;
pub(crate) mod file;
pub(crate) mod multi_port;
pub(crate) mod output;
pub(crate) mod serde_support;
pub(crate) mod utils;

// Public modules for advanced usage
pub mod cc_manager;
pub mod cc_mapping;
pub mod output_collector;

#[cfg(feature = "midi-io")]
pub(crate) mod input;

#[cfg(feature = "mpe")]
pub(crate) mod mpe;

#[cfg(feature = "midi2")]
pub(crate) mod midi2;
