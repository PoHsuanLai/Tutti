//! MIDI I/O subsystem for Tutti audio engine.
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
//! use tutti_midi_io::MidiSystem;
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

pub mod error;
pub use error::{Error, Result};

mod system;
pub use system::{MidiSystem, MidiSystemBuilder};

// Fluent builder and handle
mod midi_builder;
mod midi_handle;
pub use midi_builder::MidiBuilder;
pub use midi_handle::MidiHandle;

// Sub-handles (feature-gated)
#[cfg(feature = "midi2")]
pub use system::Midi2Handle;
#[cfg(feature = "mpe")]
pub use system::MpeHandle;

pub(crate) mod event;
pub use event::{MidiEvent, RawMidiEvent};

// Re-export essential upstream types from tutti-midi (users shouldn't need to import midi-msg directly)
pub use tutti_midi::{
    Channel, ChannelModeMsg, ChannelVoiceMsg, ControlChange, MidiMsg, SystemCommonMsg,
    SystemRealTimeMsg,
};

pub(crate) mod port;
pub use port::PortInfo;

#[cfg(feature = "midi-io")]
pub(crate) mod io;

#[cfg(feature = "midi-io")]
pub use io::{MidiInputDevice, MidiOutputDevice, MidiOutputMessage};

pub mod cc;
pub use cc::{
    CCMapping, CCMappingManager, CCNumber, CCProcessResult, CCTarget, MappingId, MidiChannel,
};

pub mod output_collector;
pub use output_collector::{
    midi_output_channel, midi_output_channel_with_capacity, MidiOutputAggregator,
    MidiOutputConsumer, MidiOutputProducer,
};

pub(crate) mod file;
pub use file::{MidiEventType, ParsedMidiFile, TimedMidiEvent};

pub mod note;
pub use note::Note;

pub use tutti_midi::{gain_to_velocity, hz_to_note, note_to_hz, velocity_to_gain};

#[cfg(feature = "mpe")]
pub(crate) mod mpe;

#[cfg(feature = "mpe")]
pub use mpe::{MpeMode, MpeZone, MpeZoneConfig};

#[cfg(feature = "midi2")]
pub(crate) mod midi2;

#[cfg(feature = "midi2")]
pub use event::UnifiedMidiEvent;

#[cfg(feature = "midi2")]
pub use midi2::{Midi2Event, Midi2MessageType};
