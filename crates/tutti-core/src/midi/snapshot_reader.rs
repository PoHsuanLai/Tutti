//! Export-mode MIDI source backed by [`MidiSnapshot`] + [`ExportTimeline`].

use super::snapshot::MidiSnapshot;
use super::source::MidiSource;
use crate::compat::Arc;
use crate::lockfree::AtomicDouble;
use crate::transport::ExportTimeline;
use tutti_midi::MidiEvent;

/// Export-mode MIDI source that reads from a snapshot based on transport beat.
///
/// Wraps a [`MidiSnapshot`] and an [`ExportTimeline`] to provide the same
/// `poll_into()` interface as `MidiRegistry`, but for offline rendering.
///
/// Each call to `poll_into` reads the current beat from the timeline,
/// polls events in `[last_beat, current_beat)`, and advances the internal
/// cursor. The caller (ExportBuilder) is responsible for advancing the
/// timeline between calls.
pub struct MidiSnapshotReader {
    snapshot: MidiSnapshot,
    timeline: Arc<ExportTimeline>,
    last_poll_beat: AtomicDouble,
}

impl MidiSnapshotReader {
    pub fn new(snapshot: MidiSnapshot, timeline: Arc<ExportTimeline>) -> Self {
        let start_beat = timeline.current_beat();
        Self {
            snapshot,
            timeline,
            last_poll_beat: AtomicDouble::new(start_beat),
        }
    }
}

impl MidiSource for MidiSnapshotReader {
    fn poll_into(&self, unit_id: u64, buffer: &mut [MidiEvent]) -> usize {
        let current_beat = self.timeline.current_beat();
        let last_beat = self.last_poll_beat.get();

        // Nothing to poll if timeline hasn't advanced
        if current_beat <= last_beat {
            return 0;
        }

        let events = self.snapshot.poll_range(unit_id, last_beat, current_beat);
        let count = events.len().min(buffer.len());
        buffer[..count].copy_from_slice(&events[..count]);
        self.last_poll_beat.set(current_beat);
        count
    }
}

impl Clone for MidiSnapshotReader {
    fn clone(&self) -> Self {
        Self {
            snapshot: self.snapshot.clone(),
            timeline: Arc::clone(&self.timeline),
            last_poll_beat: AtomicDouble::new(self.last_poll_beat.get()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::ExportConfig;

    fn note_on(note: u8, vel: u8) -> MidiEvent {
        MidiEvent::note_on_builder(note, vel).build()
    }

    #[test]
    fn test_snapshot_reader_polls_on_advance() {
        let mut snapshot = MidiSnapshot::new();
        let unit_id = 42;
        snapshot.add_event(unit_id, 0.0, note_on(60, 100));
        snapshot.add_event(unit_id, 1.0, note_on(64, 100));
        snapshot.add_event(unit_id, 2.0, note_on(67, 100));

        let timeline = Arc::new(ExportTimeline::new(&ExportConfig {
            start_beat: 0.0,
            tempo: 120.0,
            sample_rate: 44100.0,
            loop_range: None,
        }));

        let reader = MidiSnapshotReader::new(snapshot, Arc::clone(&timeline));

        let mut buffer = [MidiEvent::note_on(0, 0, 0, 0); 16];

        // At beat 0, nothing yet (no advance)
        let count = reader.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 0);

        // Advance to beat 0.5 — should get event at beat 0
        let samples_per_beat = 44100.0 / 2.0; // 120 BPM
        timeline.advance((0.5 * samples_per_beat) as usize);
        let count = reader.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 1);

        // Advance to beat 1.5 — should get event at beat 1
        timeline.advance((1.0 * samples_per_beat) as usize);
        let count = reader.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 1);

        // Advance to beat 3.0 — should get event at beat 2
        timeline.advance((1.5 * samples_per_beat) as usize);
        let count = reader.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 1);

        // No more events
        timeline.advance((1.0 * samples_per_beat) as usize);
        let count = reader.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_snapshot_reader_no_events_for_unknown_unit() {
        let snapshot = MidiSnapshot::new();
        let timeline = Arc::new(ExportTimeline::new(&ExportConfig::default()));
        let reader = MidiSnapshotReader::new(snapshot, Arc::clone(&timeline));

        let mut buffer = [MidiEvent::note_on(0, 0, 0, 0); 16];
        timeline.advance(1000);
        let count = reader.poll_into(999, &mut buffer);
        assert_eq!(count, 0);
    }
}
