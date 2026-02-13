//! Fluent MIDI system handle.

use crate::system::MidiSystem;
use std::sync::Arc;

#[cfg(feature = "midi2")]
use crate::system::Midi2Handle;
#[cfg(feature = "mpe")]
use crate::system::MpeHandle;

/// Fluent handle for MIDI operations. Methods are no-ops when MIDI is disabled.
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
    pub fn new(midi: Option<Arc<MidiSystem>>) -> Self {
        Self { midi }
    }

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

    /// # Example
    /// ```ignore
    /// let bend = engine.midi().mpe().pitch_bend(60);  // Returns 0.0 if disabled
    /// ```
    #[cfg(feature = "mpe")]
    pub fn mpe(&self) -> MpeHandle {
        if let Some(ref midi) = self.midi {
            midi.mpe()
        } else {
            MpeHandle::new(None)
        }
    }

    /// # Example
    /// ```ignore
    /// let event = engine.midi().midi2().note_on(60, 0.8, 0);
    /// ```
    #[cfg(feature = "midi2")]
    pub fn midi2(&self) -> Midi2Handle {
        Midi2Handle
    }

    /// No-op when MIDI is disabled.
    #[cfg(feature = "midi-io")]
    pub fn connect_device_by_name(&self, name: &str) -> crate::Result<()> {
        if let Some(ref midi) = self.midi {
            midi.connect_device_by_name(name)
        } else {
            Ok(())
        }
    }

    /// Returns empty list when MIDI is disabled.
    #[cfg(feature = "midi-io")]
    pub fn list_devices(&self) -> Vec<crate::MidiInputDevice> {
        if let Some(ref midi) = self.midi {
            midi.list_devices()
        } else {
            Vec::new()
        }
    }

    /// No-op when MIDI is disabled.
    #[cfg(feature = "midi-io")]
    pub fn disconnect_device(&self) {
        if let Some(ref midi) = self.midi {
            midi.disconnect_device();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.midi.is_some()
    }

    /// Direct access to the underlying `MidiSystem` for advanced operations.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_handle() {
        let handle = MidiHandle::new(None);
        assert!(!handle.is_enabled());
        assert!(handle.inner().is_none());
    }

    #[test]
    fn test_enabled_handle() {
        let midi = MidiSystem::builder().build().unwrap();
        let handle = MidiHandle::new(Some(Arc::new(midi)));
        assert!(handle.is_enabled());
        assert!(handle.inner().is_some());
    }

    #[test]
    fn test_clone_shares_state() {
        let midi = Arc::new(MidiSystem::builder().build().unwrap());
        let handle1 = MidiHandle::new(Some(midi.clone()));
        let handle2 = handle1.clone();

        // Both handles should point to the same system
        assert!(Arc::ptr_eq(
            handle1.inner().unwrap(),
            handle2.inner().unwrap()
        ));
    }
}
