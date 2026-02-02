//! MIDI-aware AudioUnit trait
//!
//! Defines the trait for audio nodes that can process MIDI events alongside audio.
//! This allows TuttiNet to route MIDI events to nodes that support it.

use crate::compat::Vec;
use fundsp::prelude::AudioUnit;

/// MIDI event type re-exported from tutti-midi-io
/// This avoids tutti-core depending directly on tutti-midi-io for the trait definition
pub type MidiEvent = tutti_midi_io::MidiEvent;

/// Trait for audio units that can process MIDI events alongside audio.
///
/// This extends the standard AudioUnit trait to support MIDI input/output.
/// Nodes that implement this trait can receive MIDI events during graph processing.
///
/// # Example
///
/// ```ignore
/// struct MyPlugin {
///     pending_midi: Vec<MidiEvent>,
/// }
///
/// impl AudioUnit for MyPlugin {
///     fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
///         // Process audio + consume self.pending_midi
///     }
/// }
///
/// impl MidiAudioUnit for MyPlugin {
///     fn queue_midi(&mut self, events: &[MidiEvent]) {
///         self.pending_midi.extend_from_slice(events);
///     }
///
///     fn has_midi_output(&self) -> bool {
///         false  // This plugin doesn't generate MIDI
///     }
/// }
/// ```
pub trait MidiAudioUnit: AudioUnit {
    /// Queue MIDI events to be processed in the next audio callback.
    ///
    /// This is called by TuttiNet before calling `AudioUnit::process()`.
    /// The events should be consumed during the next `process()` call.
    ///
    /// # Arguments
    /// * `events` - Slice of MIDI events with frame offsets relative to the current buffer
    ///
    /// # RT-Safety
    /// This method should be RT-safe:
    /// - No allocations (pre-allocate event buffers)
    /// - No blocking operations
    /// - Fast copying or zero-copy where possible
    fn queue_midi(&mut self, events: &[MidiEvent]);

    /// Returns true if this node produces MIDI output.
    ///
    /// If true, TuttiNet will call `collect_midi_output()` after processing
    /// to route MIDI to downstream nodes.
    fn has_midi_output(&self) -> bool {
        false // Default: most nodes don't generate MIDI
    }

    /// Collect MIDI events generated during the last `process()` call.
    ///
    /// Only called if `has_midi_output()` returns true.
    /// The returned events should have frame offsets relative to the processed buffer.
    ///
    /// # Returns
    /// Vector of MIDI events generated during processing
    ///
    /// # RT-Safety
    /// Should avoid allocations where possible (reuse buffers internally)
    fn collect_midi_output(&mut self) -> Vec<MidiEvent> {
        Vec::new() // Default: no output
    }

    /// Clear any queued MIDI events.
    ///
    /// Called when the audio graph is reset or restarted.
    fn clear_midi(&mut self) {
        // Default: no-op (override if you buffer MIDI)
    }
}

/// Helper trait for dynamic dispatch
///
/// Allows TuttiNet to check at runtime if a node supports MIDI.
pub trait AsMidiAudioUnit {
    /// Try to downcast to MidiAudioUnit
    ///
    /// Instead of returning a reference, this calls a closure with the MIDI node.
    /// This avoids lifetime issues with trait objects.
    fn with_midi_audio_unit<F, R>(&mut self, f: F) -> Option<R>
    where
        F: for<'a> FnOnce(&'a mut dyn MidiAudioUnit) -> R;
}

/// Blanket implementation: all MidiAudioUnit implementors can be queried
impl<T: MidiAudioUnit> AsMidiAudioUnit for T {
    fn with_midi_audio_unit<F, R>(&mut self, f: F) -> Option<R>
    where
        F: for<'a> FnOnce(&'a mut dyn MidiAudioUnit) -> R,
    {
        Some(f(self))
    }
}

/// Default implementation: regular AudioUnit nodes don't support MIDI
impl AsMidiAudioUnit for dyn AudioUnit {
    fn with_midi_audio_unit<F, R>(&mut self, _f: F) -> Option<R>
    where
        F: for<'a> FnOnce(&'a mut dyn MidiAudioUnit) -> R,
    {
        None
    }
}
