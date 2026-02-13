//! MIDI I/O subsystem for Tutti audio engine.
//!
//! Provides port management, hardware I/O, MPE, MIDI 2.0, CC mapping, and output collection.
//!
//! Feature gates: `midi-io` (hardware I/O), `mpe` (polyphonic expression), `midi2` (high-res messages).

pub mod error;
pub use error::{Error, Result};

mod system;
pub use system::{MidiSystem, MidiSystemBuilder};

mod midi_builder;
mod midi_handle;
pub use midi_builder::MidiBuilder;
pub use midi_handle::MidiHandle;

#[cfg(feature = "midi2")]
pub use system::Midi2Handle;
#[cfg(feature = "mpe")]
pub use system::MpeHandle;

pub(crate) mod event;
pub use event::{MidiEvent, RawMidiEvent};

pub use tutti_midi::{
    Channel, ChannelModeMsg, ChannelVoiceMsg, ControlChange, MidiMsg, SystemCommonMsg,
    SystemRealTimeMsg,
};

pub(crate) mod port;
pub use port::{PortInfo, PortType};

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

pub use tutti_midi::note;
pub use tutti_midi::Note;

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
