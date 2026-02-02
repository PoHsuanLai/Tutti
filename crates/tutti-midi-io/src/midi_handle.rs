//! Fluent MIDI system handle
//!
//! Provides a fluent API for MIDI operations that works regardless of whether
//! the MIDI subsystem is enabled. Methods are no-ops when MIDI is disabled.

use crate::system::MidiSystem;
use std::sync::Arc;

#[cfg(feature = "midi2")]
use crate::system::Midi2Handle;
#[cfg(feature = "mpe")]
use crate::system::MpeHandle;

/// Fluent handle for MIDI operations.
///
/// This handle wraps an optional MIDI system and provides a fluent API that
/// works whether or not MIDI is enabled. When MIDI is disabled, methods are no-ops.
///
/// # Example
/// ```ignore
/// // Always works, even if MIDI not enabled
/// engine.midi().send().note_on(0, 60, 100);
///
/// // Access sub-handles (always returns handles)
/// let bend = engine.midi().mpe().pitch_bend(60);  // Returns 0.0 if disabled
/// let event = engine.midi().midi2().note_on(60, 0.8, 0);
/// ```
pub struct MidiHandle {
    midi: Option<Arc<MidiSystem>>,
}

impl MidiHandle {
    /// Create a new handle
    pub fn new(midi: Option<Arc<MidiSystem>>) -> Self {
        Self { midi }
    }

    /// Get fluent MIDI output builder.
    ///
    /// Returns a builder for chaining MIDI output messages.
    /// When MIDI is disabled, the builder methods are no-ops.
    ///
    /// # Example
    /// ```ignore
    /// midi.send()
    ///     .note_on(0, 60, 100)
    ///     .cc(0, 74, 64)
    ///     .pitch_bend(0, 0);
    /// ```
    #[cfg(feature = "midi-io")]
    pub fn send(&self) -> crate::midi_builder::MidiBuilder<'_> {
        crate::midi_builder::MidiBuilder::new(self.midi.as_deref())
    }

    /// Get MPE handle for per-note expression.
    ///
    /// Returns a handle that works whether or not MPE is enabled.
    /// Methods are no-ops or return defaults when MPE is disabled.
    ///
    /// # Example
    /// ```ignore
    /// // Always works - no Option<> unwrapping needed
    /// let bend = engine.midi().mpe().pitch_bend(60);  // Returns 0.0 if disabled
    /// ```
    #[cfg(feature = "mpe")]
    pub fn mpe(&self) -> MpeHandle {
        if let Some(ref midi) = self.midi {
            midi.mpe()
        } else {
            // Return disabled handle when MIDI subsystem not enabled
            MpeHandle::new(None)
        }
    }

    /// Get MIDI 2.0 handle for high-resolution messages.
    ///
    /// Returns a handle that creates MIDI 2.0 events.
    /// Always available (zero-cost abstraction).
    ///
    /// # Example
    /// ```ignore
    /// let event = engine.midi().midi2().note_on(60, 0.8, 0);
    /// ```
    #[cfg(feature = "midi2")]
    pub fn midi2(&self) -> Midi2Handle {
        // Always return handle - it's a ZST, zero cost
        Midi2Handle
    }

    /// Connect to a MIDI input device by name.
    ///
    /// Returns `Ok(())` when MIDI is disabled (no-op).
    #[cfg(feature = "midi-io")]
    pub fn connect_device_by_name(&self, name: &str) -> crate::Result<()> {
        if let Some(ref midi) = self.midi {
            midi.connect_device_by_name(name)
        } else {
            Ok(()) // No-op when MIDI disabled
        }
    }

    /// List available MIDI input devices.
    ///
    /// Returns empty vector when MIDI is disabled.
    #[cfg(feature = "midi-io")]
    pub fn list_devices(&self) -> Vec<crate::MidiInputDevice> {
        if let Some(ref midi) = self.midi {
            midi.list_devices()
        } else {
            Vec::new()
        }
    }

    /// Disconnect from current MIDI input device.
    ///
    /// No-op when MIDI is disabled.
    #[cfg(feature = "midi-io")]
    pub fn disconnect_device(&self) {
        if let Some(ref midi) = self.midi {
            midi.disconnect_device();
        }
    }

    /// Check if MIDI subsystem is enabled.
    pub fn is_enabled(&self) -> bool {
        self.midi.is_some()
    }

    /// Get reference to inner MidiSystem if enabled.
    ///
    /// Useful for advanced operations not covered by the fluent API.
    pub fn inner(&self) -> Option<&Arc<MidiSystem>> {
        self.midi.as_ref()
    }
}

impl Clone for MidiHandle {
    fn clone(&self) -> Self {
        Self {
            midi: self.midi.clone(),
        }
    }
}
