//! MIDI port management for internal routing.
//!
//! This module provides virtual MIDI ports for routing events between
//! different parts of the system (synths, effects, sequencer, etc.).

pub(crate) mod async_port;
mod manager;

pub use manager::{MidiPortManager, PortInfo, PortType};
