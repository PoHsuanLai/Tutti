//! Export context for offline audio rendering.
//!
//! Provides isolated timeline and MIDI snapshot for export operations.
//! This ensures exports don't interfere with live audio playback.

use crate::compat::Arc;
use crate::transport::{ExportConfig, ExportTimeline};

#[cfg(feature = "midi")]
use crate::midi::MidiSnapshot;

/// Context for offline audio export.
///
/// Contains all state needed to render audio independently of the
/// live transport and MIDI registry:
///
/// - `timeline`: Simulated transport that advances by sample count
/// - `midi_snapshot`: Non-destructive copy of MIDI events (if midi feature enabled)
///
/// # Example
/// ```ignore
/// let context = ExportContext::new(ExportConfig {
///     start_beat: 0.0,
///     tempo: 120.0,
///     sample_rate: 44100.0,
///     loop_range: None,
/// });
///
/// // During render, advance timeline per sample
/// for _ in 0..total_samples {
///     context.timeline.advance(1);
///     // Synths/automation read from context.timeline
/// }
/// ```
#[derive(Debug)]
pub struct ExportContext {
    /// Simulated transport timeline.
    pub timeline: Arc<ExportTimeline>,

    /// MIDI event snapshot for non-destructive playback.
    #[cfg(feature = "midi")]
    pub midi_snapshot: MidiSnapshot,
}

impl ExportContext {
    /// Create a new export context from configuration.
    #[cfg(feature = "midi")]
    pub fn new(config: ExportConfig) -> Self {
        Self {
            timeline: Arc::new(ExportTimeline::new(&config)),
            midi_snapshot: MidiSnapshot::new(),
        }
    }

    /// Create a new export context from configuration.
    #[cfg(not(feature = "midi"))]
    pub fn new(config: ExportConfig) -> Self {
        Self {
            timeline: Arc::new(ExportTimeline::new(&config)),
        }
    }

    /// Create export context with a pre-built MIDI snapshot.
    #[cfg(feature = "midi")]
    pub fn with_midi_snapshot(config: ExportConfig, midi_snapshot: MidiSnapshot) -> Self {
        Self {
            timeline: Arc::new(ExportTimeline::new(&config)),
            midi_snapshot,
        }
    }

    /// Get the export timeline.
    pub fn timeline(&self) -> &Arc<ExportTimeline> {
        &self.timeline
    }

    /// Get mutable reference to MIDI snapshot for adding events.
    #[cfg(feature = "midi")]
    pub fn midi_snapshot_mut(&mut self) -> &mut MidiSnapshot {
        &mut self.midi_snapshot
    }

    /// Reset context for re-rendering.
    ///
    /// Resets timeline to start beat and MIDI snapshot cursors.
    pub fn reset(&mut self, start_beat: f64) {
        self.timeline.reset(start_beat);

        #[cfg(feature = "midi")]
        self.midi_snapshot.reset();
    }
}

impl Clone for ExportContext {
    fn clone(&self) -> Self {
        Self {
            timeline: self.timeline.clone(),
            #[cfg(feature = "midi")]
            midi_snapshot: self.midi_snapshot.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_export_context_reset() {
        let mut context = ExportContext::new(ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        // Advance timeline
        context.timeline.advance(44100); // 1 second = 2 beats at 120 BPM

        assert!((context.timeline.current_beat() - 2.0).abs() < 0.01);

        // Reset to beat 0
        context.reset(0.0);
        assert!((context.timeline.current_beat() - 0.0).abs() < 0.001);
    }

    #[cfg(feature = "midi")]
    #[test]
    fn test_export_context_with_midi_snapshot() {
        use tutti_midi::MidiEvent;

        let mut snapshot = MidiSnapshot::new();
        snapshot.add_event(1, 0.0, MidiEvent::note_on_builder(60, 100).build());

        let context = ExportContext::with_midi_snapshot(
            ExportConfig {
                start_beat: 0.0,
                tempo: 120.0,
                sample_rate: 44100.0,
                loop_range: None,
            },
            snapshot,
        );

        assert!(context.midi_snapshot.has_events(1));
    }

    #[cfg(feature = "midi")]
    #[test]
    fn test_export_context_midi_snapshot_mut() {
        use tutti_midi::MidiEvent;

        let mut context = ExportContext::new(ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        });

        // Add events via mutable reference
        context
            .midi_snapshot_mut()
            .add_event(1, 0.0, MidiEvent::note_on_builder(60, 100).build());

        assert!(context.midi_snapshot.has_events(1));
    }
}
