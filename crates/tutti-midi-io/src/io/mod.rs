//! Hardware MIDI I/O.
//!
//! Device enumeration, connection, and real-time I/O via midir.
//! Requires the `midi-io` feature.

mod input;
mod output;

pub use input::{MidiInputDevice, MidiInputManager};
pub use output::{MidiOutputDevice, MidiOutputManager, MidiOutputMessage};
