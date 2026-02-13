//! Unified MIDI event sourcing for AudioUnits.
//!
//! The [`MidiSource`] trait provides a single abstraction for pulling MIDI
//! events in the audio thread, covering both live and export paths:
//!
//! - **Live**: [`MidiRegistry`] — destructive read from lock-free SPSC channels
//! - **Export**: [`MidiSnapshotReader`] — cursor-based read from [`MidiSnapshot`]

use tutti_midi::MidiEvent;

/// Trait for pulling MIDI events in the audio thread.
///
/// AudioUnits store `Option<Box<dyn MidiSource>>` and call `poll_into()`
/// during `tick()`/`process()`. The concrete implementation determines
/// whether events come from the live registry or an export snapshot.
pub trait MidiSource: Send + Sync {
    /// Poll available MIDI events for the given unit into the buffer.
    ///
    /// Returns the number of events written to `buffer`.
    /// Zero allocations, no blocking — safe for the audio thread.
    fn poll_into(&self, unit_id: u64, buffer: &mut [MidiEvent]) -> usize;
}
