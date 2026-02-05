//! Abstraction for MIDI input sources.
//!
//! This trait allows the audio callback to read MIDI events from various sources
//! (hardware ports, virtual ports, etc.) without depending on specific implementations.

use crate::MidiEvent;

/// RT-safe MIDI input source that can be polled from the audio callback.
///
/// Implementations must be lock-free and safe to call from the audio thread.
/// The `cycle_read` method is called once per audio buffer to collect all
/// pending MIDI events.
///
/// # Safety
///
/// All methods must be RT-safe:
/// - No heap allocations
/// - No locks (except lock-free atomics)
/// - No blocking I/O
/// - O(n) where n is the number of events, not unbounded
pub trait MidiInputSource: Send + Sync {
    /// Read all pending MIDI events for this audio cycle.
    ///
    /// Returns a slice of (port_index, event) tuples. The slice is valid until
    /// the next call to `cycle_read`.
    ///
    /// # Arguments
    /// * `nframes` - Number of audio frames in this cycle (for timing context)
    ///
    /// # RT Safety
    /// This method is called from the audio thread and must be lock-free.
    fn cycle_read(&self, nframes: usize) -> &[(usize, MidiEvent)];

    /// Check if any input ports are active.
    ///
    /// Can be used to skip MIDI processing when no ports are connected.
    fn has_active_inputs(&self) -> bool {
        true
    }
}

/// A no-op MIDI input source for when MIDI is disabled.
#[derive(Debug, Default)]
pub struct NoMidiInput;

impl MidiInputSource for NoMidiInput {
    fn cycle_read(&self, _nframes: usize) -> &[(usize, MidiEvent)] {
        &[]
    }

    fn has_active_inputs(&self) -> bool {
        false
    }
}
