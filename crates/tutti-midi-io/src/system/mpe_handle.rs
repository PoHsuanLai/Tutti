//! MPE sub-handle for per-note expression access.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::mpe::{MpeMode, MpeProcessor, PerNoteExpression};

pub struct MpeHandle {
    processor: Option<Arc<RwLock<MpeProcessor>>>,
}

impl MpeHandle {
    pub(crate) fn new(processor: Option<Arc<RwLock<MpeProcessor>>>) -> Self {
        Self { processor }
    }

    pub fn expression(&self) -> Option<Arc<PerNoteExpression>> {
        self.processor.as_ref().map(|p| p.read().expression())
    }

    /// Combined per-note + global pitch bend, normalized to -1.0..1.0.
    #[inline]
    pub fn pitch_bend(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend(note))
            .unwrap_or(0.0)
    }

    /// Per-note only (excludes global/master channel bend).
    #[inline]
    pub fn pitch_bend_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend_per_note(note))
            .unwrap_or(0.0)
    }

    #[inline]
    pub fn pitch_bend_global(&self) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pitch_bend_global())
            .unwrap_or(0.0)
    }

    /// Max of per-note and global pressure, normalized to 0.0..1.0.
    #[inline]
    pub fn pressure(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pressure(note))
            .unwrap_or(0.0)
    }

    #[inline]
    pub fn pressure_per_note(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_pressure_per_note(note))
            .unwrap_or(0.0)
    }

    /// CC74 slide, normalized to 0.0..1.0.
    #[inline]
    pub fn slide(&self, note: u8) -> f32 {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().get_slide(note))
            .unwrap_or(0.5)
    }

    #[inline]
    pub fn is_note_active(&self, note: u8) -> bool {
        self.processor
            .as_ref()
            .map(|p| p.read().expression().is_active(note))
            .unwrap_or(false)
    }

    pub fn mode(&self) -> MpeMode {
        self.processor
            .as_ref()
            .map(|p| p.read().mode().clone())
            .unwrap_or(MpeMode::Disabled)
    }

    pub fn is_enabled(&self) -> bool {
        self.processor
            .as_ref()
            .map(|p| !matches!(p.read().mode(), MpeMode::Disabled))
            .unwrap_or(false)
    }

    /// Feed an external event into the MPE processor (MIDI 1.0 or 2.0).
    #[cfg(feature = "midi2")]
    pub fn process_event(&self, event: &crate::UnifiedMidiEvent) {
        if let Some(ref p) = self.processor {
            p.write().process_unified(event);
        }
    }

    /// Call before sending a Note On to allocate an MPE member channel.
    /// Returns None if MPE is disabled or no channels are free.
    pub fn allocate_channel(&self, note: u8) -> Option<u8> {
        self.processor
            .as_ref()
            .and_then(|p| p.write().allocate_channel_for_note(note))
    }

    /// Frees the member channel for reuse by other notes.
    pub fn release_channel(&self, note: u8) {
        if let Some(ref p) = self.processor {
            p.write().release_channel_for_note(note);
        }
    }

    pub fn get_channel(&self, note: u8) -> Option<u8> {
        self.processor
            .as_ref()
            .and_then(|p| p.read().get_channel_for_note(note))
    }

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

    /// Clears all channel allocations and resets expression values.
    /// Call when stopping playback or changing MPE configuration.
    pub fn reset(&self) {
        if let Some(ref p) = self.processor {
            p.write().reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mpe::{MpeMode, MpeZoneConfig};

    #[test]
    fn test_disabled_handle_defaults() {
        let handle = MpeHandle::new(None);

        // All reads should return safe defaults
        assert_eq!(handle.pitch_bend(60), 0.0);
        assert_eq!(handle.pitch_bend_per_note(60), 0.0);
        assert_eq!(handle.pitch_bend_global(), 0.0);
        assert_eq!(handle.pressure(60), 0.0);
        assert_eq!(handle.pressure_per_note(60), 0.0);
        assert_eq!(handle.slide(60), 0.5); // slide defaults to 0.5
        assert!(!handle.is_note_active(60));
        assert!(!handle.is_enabled());
        assert!(matches!(handle.mode(), MpeMode::Disabled));
        assert!(!handle.has_lower_zone());
        assert!(!handle.has_upper_zone());
        assert!(handle.expression().is_none());
        assert!(handle.allocate_channel(60).is_none());
        assert!(handle.get_channel(60).is_none());
    }

    #[test]
    fn test_enabled_expression_reads() {
        let processor = Arc::new(RwLock::new(
            MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15))),
        ));
        let handle = MpeHandle::new(Some(processor.clone()));

        assert!(handle.is_enabled());

        // Process a note on via the processor directly (simulating MIDI input)
        {
            let mut p = processor.write();
            let note_on = crate::MidiEvent::note_on(0, 1, 60, 100); // Channel 1 = member
            p.process_midi1(&note_on);
        }

        assert!(handle.is_note_active(60));

        // Process pitch bend on the same member channel
        {
            let mut p = processor.write();
            let bend = crate::MidiEvent::pitch_bend(0, 1, 16383); // Max bend
            p.process_midi1(&bend);
        }

        let bend = handle.pitch_bend(60);
        assert!((bend - 1.0).abs() < 0.01, "Expected ~1.0, got {bend}");
    }

    #[test]
    fn test_channel_allocation_and_release() {
        let processor = Arc::new(RwLock::new(
            MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(5))),
        ));
        let handle = MpeHandle::new(Some(processor));

        // Allocate a channel for note 60
        let ch = handle.allocate_channel(60);
        assert!(ch.is_some(), "Should allocate a member channel");
        let ch = ch.unwrap();
        assert!(ch >= 1 && ch <= 5, "Channel should be in member range 1-5, got {ch}");

        // Should be able to look up the channel
        let found = handle.get_channel(60);
        assert_eq!(found, Some(ch));

        // Release and verify it's gone
        handle.release_channel(60);
        assert_eq!(handle.get_channel(60), None);
    }

    #[test]
    fn test_zone_detection() {
        // Lower zone only
        let proc_lower = Arc::new(RwLock::new(
            MpeProcessor::new(MpeMode::LowerZone(MpeZoneConfig::lower(15))),
        ));
        let handle = MpeHandle::new(Some(proc_lower));
        assert!(handle.has_lower_zone());
        assert!(!handle.has_upper_zone());

        // Upper zone only
        let proc_upper = Arc::new(RwLock::new(
            MpeProcessor::new(MpeMode::UpperZone(MpeZoneConfig::upper(5))),
        ));
        let handle = MpeHandle::new(Some(proc_upper));
        assert!(!handle.has_lower_zone());
        assert!(handle.has_upper_zone());

        // Dual zone
        let proc_dual = Arc::new(RwLock::new(MpeProcessor::new(MpeMode::DualZone {
            lower: MpeZoneConfig::lower(7),
            upper: MpeZoneConfig::upper(7),
        })));
        let handle = MpeHandle::new(Some(proc_dual));
        assert!(handle.has_lower_zone());
        assert!(handle.has_upper_zone());
    }
}
