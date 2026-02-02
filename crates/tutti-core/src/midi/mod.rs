//! MIDI support for Tutti audio graph.
//!
//! This module provides MIDI traits and routing infrastructure:
//! - [`traits`]: MIDI-aware AudioUnit traits
//! - [`registry`]: RT-safe MIDI event routing system

pub mod registry;
pub mod traits;

// Re-export commonly used types
pub use registry::MidiRegistry;
pub use traits::{AsMidiAudioUnit, MidiAudioUnit, MidiEvent};
