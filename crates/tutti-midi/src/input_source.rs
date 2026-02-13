//! Abstraction for MIDI input sources.
//!
//! This trait allows the audio callback to read MIDI events from various sources
//! (hardware ports, virtual ports, etc.) without depending on specific implementations.

use crate::MidiEvent;
#[cfg(feature = "std")]
use std::time::Instant;

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
#[cfg(feature = "std")]
pub trait MidiInputSource: Send + Sync {
    /// Returns `(port_index, event)` tuples. Valid until the next call.
    ///
    /// RT-safe: called from the audio thread, must be lock-free.
    fn cycle_read(
        &self,
        nframes: usize,
        buffer_start: Instant,
        sample_rate: f64,
    ) -> &[(usize, MidiEvent)];

    fn has_active_inputs(&self) -> bool {
        true
    }
}

/// Fallback for `no_std` (no timestamp support).
#[cfg(not(feature = "std"))]
pub trait MidiInputSource: Send + Sync {
    fn cycle_read(&self, nframes: usize) -> &[(usize, MidiEvent)];

    fn has_active_inputs(&self) -> bool {
        true
    }
}

/// No-op source for when MIDI is disabled.
#[derive(Debug, Default)]
pub struct NoMidiInput;

#[cfg(feature = "std")]
impl MidiInputSource for NoMidiInput {
    fn cycle_read(
        &self,
        _nframes: usize,
        _buffer_start: Instant,
        _sample_rate: f64,
    ) -> &[(usize, MidiEvent)] {
        &[]
    }

    fn has_active_inputs(&self) -> bool {
        false
    }
}

#[cfg(not(feature = "std"))]
impl MidiInputSource for NoMidiInput {
    fn cycle_read(&self, _nframes: usize) -> &[(usize, MidiEvent)] {
        &[]
    }

    fn has_active_inputs(&self) -> bool {
        false
    }
}
