//! MIDI CC (Control Change) mapping subsystem.
//!
//! Provides MIDI learn functionality and parameter mapping.

pub mod manager;
pub mod mapping;

pub use manager::{CCMappingManager, CCProcessResult};
pub use mapping::{CCMapping, CCNumber, CCTarget, MappingId, MidiChannel};
