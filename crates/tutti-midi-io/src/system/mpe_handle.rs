//! MPE sub-handle for per-note expression access.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::mpe::{MpeMode, MpeProcessor, PerNoteExpression};

/// Handle for MPE functionality
pub struct MpeHandle {
    processor: Option<Arc<RwLock<MpeProcessor>>>,
}

impl MpeHandle {
    /// Create a new MPE handle (internal use only)
    pub(crate) fn new(processor: Option<Arc<RwLock<MpeProcessor>>>) -> Self {
        Self { processor }
    }

    /// Get the shared per-note expression state
    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.processor.as_ref().map(|p| p.read().expression())
    }

    /// Get pitch bend for a note (combined per-note + global)
    ///
    /// Returns normalized value: -1.0 (max down) to 1.0 (max up)
    #[inline]
    pub fn pitch_bend(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend(note))
            .unwrap_or(0.0)
    }

    /// Get per-note pitch bend only (without global)
    #[inline]
    pub fn pitch_bend_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend_per_note(note))
            .unwrap_or(0.0)
    }

    /// Get global pitch bend (from master channel)
    #[inline]
    pub fn pitch_bend_global(&self) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend_global())
            .unwrap_or(0.0)
    }

    /// Get pressure for a note (max of per-note and global)
    ///
    /// Returns normalized value: 0.0 to 1.0
    #[inline]
    pub fn pressure(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pressure(note))
            .unwrap_or(0.0)
    }

    /// Get per-note pressure only
    #[inline]
    pub fn pressure_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pressure_per_note(note))
            .unwrap_or(0.0)
    }

    /// Get slide (CC74) for a note
    ///
    /// Returns normalized value: 0.0 to 1.0
    #[inline]
    pub fn slide(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_slide(note))
            .unwrap_or(0.5)
    }

    /// Check if a note is currently active
    #[inline]
    pub fn is_note_active(&self, note: u8) -> bool {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().is_active(note))
            .unwrap_or(false)
    }

    /// Get the current MPE mode
    pub fn mode(&self) -> MpeMode {
        self.processor
            .as_ref()
            .map(|p| p.read().mode().clone())
            .unwrap_or(MpeMode::Disabled)
    }

    /// Check if MPE is enabled
    pub fn is_enabled(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| !matches!(p.read().mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    // ========================================================================
    // Incoming: Process unified MIDI events
    // ========================================================================

    /// Process a unified MIDI event (MIDI 1.0 or 2.0)
    ///
    /// Dispatches to the appropriate handler based on the event type.
    /// This allows external callers to feed events into the MPE processor.
    #[cfg(feature = "midi2")]
    pub fn process_event(&self, event: &crate::UnifiedMidiEvent) {
        if let Some(ref p) = self.processor {
            p.write().process_unified(event);
        }
    }

    // ========================================================================
    // Outgoing MPE: Send notes with automatic channel allocation
    // ========================================================================

    /// Allocate a channel for outgoing MPE note
    ///
    /// Call this before sending a Note On to allocate an MPE channel.
    /// Returns the channel to use, or None if MPE is disabled.
    ///
    /// # Example
    /// ```ignore
    /// if let Some(channel) = midi.mpe().allocate_channel(60) {
    ///     midi.send_note_on(channel, 60, 100)?;
    /// }
    /// ```
    pub fn allocate_channel(&self, note: u8) -> Option<u8> {
        self.processor
            .as_ref()
            .and_then(|p| p.write().allocate_channel_for_note(note))
    }

    /// Release a channel after Note Off
    ///
    /// Frees the channel for reuse by other notes.
    pub fn release_channel(&self, note: u8) {
        if let Some(ref p) = self.processor {
            p.write().release_channel_for_note(note);
        }
    }

    /// Get the channel currently assigned to a note
    ///
    /// Returns None if the note is not currently playing or MPE is disabled.
    pub fn get_channel(&self, note: u8) -> Option<u8> {
        self.processor
            .as_ref()
            .and_then(|p| p.read().get_channel_for_note(note))
    }

    /// Check if using lower zone
    pub fn has_lower_zone(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| {
                matches!(
                    p.read().mode(),
                    MpeMode::LowerZone(_) | MpeMode::DualZone { .. }
                )
            })
            .unwrap_or(false)
    }

    /// Check if using upper zone
    pub fn has_upper_zone(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| {
                matches!(
                    p.read().mode(),
                    MpeMode::UpperZone(_) | MpeMode::DualZone { .. }
                )
            })
            .unwrap_or(false)
    }

    /// Reset all MPE state
    ///
    /// Clears all channel allocations and resets expression values.
    /// Call this when stopping playback or changing MPE configuration.
    pub fn reset(&self) {
        if let Some(ref p) = self.processor {
            p.write().reset();
        }
    }
}
