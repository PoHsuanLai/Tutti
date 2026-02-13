//! Non-destructive MIDI event storage for offline export.
//!
//! Unlike the real-time MIDI registry which uses destructive reads,
//! MidiSnapshot allows events to be polled multiple times without
//! consuming them.

use crate::compat::{HashMap, Vec};
use crate::{AtomicUsize, Ordering};
use tutti_midi::MidiEvent;

#[derive(Debug, Clone)]
pub struct TimedMidiEvent {
    pub event: MidiEvent,
    /// Beat position when this event should trigger.
    pub beat: f64,
}

/// Non-destructive snapshot of MIDI events for export.
///
/// Events are stored per unit ID and sorted by beat position.
/// Polling advances a cursor but doesn't remove events, allowing
/// the same snapshot to be used for multiple renders.
///
/// # Example
/// ```ignore
/// let mut snapshot = MidiSnapshot::new();
/// snapshot.add_event(synth_id, 0.0, note_on(60, 100));
/// snapshot.add_event(synth_id, 1.0, note_off(60));
///
/// // First poll gets note_on
/// let events = snapshot.poll_range(synth_id, 0.0, 0.5);
/// assert_eq!(events.len(), 1);
///
/// // Reset and poll again - same events
/// snapshot.reset();
/// let events = snapshot.poll_range(synth_id, 0.0, 0.5);
/// assert_eq!(events.len(), 1);
/// ```
/// Note: `poll_range` uses atomic cursors so it can take `&self` (RT-safe).
#[derive(Debug, Default)]
pub struct MidiSnapshot {
    /// Events per unit ID, sorted by beat.
    events: HashMap<u64, Vec<TimedMidiEvent>>,
    /// Current read cursor per unit (index into events vec).
    /// Atomic so `poll_range` can advance without `&mut self`.
    cursors: HashMap<u64, AtomicUsize>,
}

impl Clone for MidiSnapshot {
    fn clone(&self) -> Self {
        let cursors = self
            .cursors
            .iter()
            .map(|(&k, v)| (k, AtomicUsize::new(v.load(Ordering::Relaxed))))
            .collect();
        Self {
            events: self.events.clone(),
            cursors,
        }
    }
}

impl MidiSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_event(&mut self, unit_id: u64, beat: f64, event: MidiEvent) {
        let events = self.events.entry(unit_id).or_default();
        events.push(TimedMidiEvent { event, beat });
        // Keep sorted by beat
        events.sort_by(|a, b| a.beat.partial_cmp(&b.beat).unwrap());
        // Ensure cursor exists for this unit
        self.cursors
            .entry(unit_id)
            .or_insert_with(|| AtomicUsize::new(0));
    }

    /// Poll events in the given beat range [start, end).
    ///
    /// Returns events that fall within the range and advances the cursor.
    /// Events are returned in beat order.
    ///
    /// Uses atomic cursors so this can be called with `&self` (RT-safe).
    pub fn poll_range(&self, unit_id: u64, start_beat: f64, end_beat: f64) -> Vec<MidiEvent> {
        let events = match self.events.get(&unit_id) {
            Some(e) => e,
            None => return Vec::new(),
        };

        let cursor = match self.cursors.get(&unit_id) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let mut pos = cursor.load(Ordering::Relaxed);
        let mut result = Vec::new();

        // Advance past any events before start_beat
        while pos < events.len() && events[pos].beat < start_beat {
            pos += 1;
        }

        // Collect events in range
        while pos < events.len() && events[pos].beat < end_beat {
            result.push(events[pos].event);
            pos += 1;
        }

        cursor.store(pos, Ordering::Relaxed);
        result
    }

    pub fn has_events(&self, unit_id: u64) -> bool {
        self.events.get(&unit_id).is_some_and(|e| !e.is_empty())
    }

    pub fn unit_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.events.keys().copied()
    }

    /// Reset all cursors to the beginning.
    ///
    /// Call this before re-rendering to replay all events.
    pub fn reset(&self) {
        for cursor in self.cursors.values() {
            cursor.store(0, Ordering::Relaxed);
        }
    }

    pub fn reset_unit(&self, unit_id: u64) {
        if let Some(cursor) = self.cursors.get(&unit_id) {
            cursor.store(0, Ordering::Relaxed);
        }
    }

    pub fn total_events(&self) -> usize {
        self.events.values().map(|v| v.len()).sum()
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.cursors.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutti_midi::MidiEvent;

    fn note_on(note: u8, vel: u8) -> MidiEvent {
        MidiEvent::note_on_builder(note, vel).build()
    }

    fn note_off(note: u8) -> MidiEvent {
        MidiEvent::note_off_builder(note).build()
    }

    #[test]
    fn test_snapshot_basic() {
        let mut snapshot = MidiSnapshot::new();
        let unit_id = 123;

        snapshot.add_event(unit_id, 0.0, note_on(60, 100));
        snapshot.add_event(unit_id, 1.0, note_off(60));

        // Poll to verify events were added
        let events = snapshot.poll_range(unit_id, 0.0, 2.0);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_snapshot_poll_range() {
        let mut snapshot = MidiSnapshot::new();
        let unit_id = 123;

        snapshot.add_event(unit_id, 0.0, note_on(60, 100));
        snapshot.add_event(unit_id, 0.5, note_on(64, 100));
        snapshot.add_event(unit_id, 1.0, note_off(60));
        snapshot.add_event(unit_id, 1.0, note_off(64));

        // Poll first half beat
        let events = snapshot.poll_range(unit_id, 0.0, 0.5);
        assert_eq!(events.len(), 1); // Just the first note_on

        // Poll next half beat
        let events = snapshot.poll_range(unit_id, 0.5, 1.0);
        assert_eq!(events.len(), 1); // Second note_on

        // Poll beat 1.0
        let events = snapshot.poll_range(unit_id, 1.0, 1.5);
        assert_eq!(events.len(), 2); // Both note_offs
    }

    #[test]
    fn test_snapshot_reset() {
        let mut snapshot = MidiSnapshot::new();
        let unit_id = 123;

        snapshot.add_event(unit_id, 0.0, note_on(60, 100));

        // First poll
        let events = snapshot.poll_range(unit_id, 0.0, 1.0);
        assert_eq!(events.len(), 1);

        // Second poll without reset - no events (cursor advanced)
        let events = snapshot.poll_range(unit_id, 0.0, 1.0);
        assert_eq!(events.len(), 0);

        // Reset and poll again
        snapshot.reset();
        let events = snapshot.poll_range(unit_id, 0.0, 1.0);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_snapshot_multiple_units() {
        let mut snapshot = MidiSnapshot::new();

        snapshot.add_event(1, 0.0, note_on(60, 100));
        snapshot.add_event(2, 0.0, note_on(72, 100));

        let events1 = snapshot.poll_range(1, 0.0, 1.0);
        let events2 = snapshot.poll_range(2, 0.0, 1.0);

        assert_eq!(events1.len(), 1);
        assert_eq!(events2.len(), 1);
    }

    #[test]
    fn test_has_events() {
        let mut snapshot = MidiSnapshot::new();

        assert!(!snapshot.has_events(1));

        snapshot.add_event(1, 0.0, note_on(60, 100));

        assert!(snapshot.has_events(1));
        assert!(!snapshot.has_events(2));
    }

    #[test]
    fn test_unit_ids() {
        let mut snapshot = MidiSnapshot::new();

        snapshot.add_event(1, 0.0, note_on(60, 100));
        snapshot.add_event(3, 0.0, note_on(64, 100));
        snapshot.add_event(5, 0.0, note_on(67, 100));

        let ids: Vec<u64> = snapshot.unit_ids().collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&1));
        assert!(ids.contains(&3));
        assert!(ids.contains(&5));
    }

    #[test]
    fn test_reset_unit() {
        let mut snapshot = MidiSnapshot::new();

        snapshot.add_event(1, 0.0, note_on(60, 100));
        snapshot.add_event(2, 0.0, note_on(72, 100));

        // Poll both units
        let _ = snapshot.poll_range(1, 0.0, 1.0);
        let _ = snapshot.poll_range(2, 0.0, 1.0);

        // Both cursors advanced, no events on re-poll
        assert_eq!(snapshot.poll_range(1, 0.0, 1.0).len(), 0);
        assert_eq!(snapshot.poll_range(2, 0.0, 1.0).len(), 0);

        // Reset only unit 1
        snapshot.reset_unit(1);

        // Unit 1 has events again, unit 2 still empty
        assert_eq!(snapshot.poll_range(1, 0.0, 1.0).len(), 1);
        assert_eq!(snapshot.poll_range(2, 0.0, 1.0).len(), 0);
    }

    #[test]
    fn test_clear() {
        let mut snapshot = MidiSnapshot::new();

        snapshot.add_event(1, 0.0, note_on(60, 100));
        snapshot.add_event(2, 0.0, note_on(72, 100));

        assert_eq!(snapshot.total_events(), 2);

        snapshot.clear();

        assert_eq!(snapshot.total_events(), 0);
        assert!(!snapshot.has_events(1));
        assert!(!snapshot.has_events(2));
    }

    #[test]
    fn test_poll_nonexistent_unit() {
        let mut snapshot = MidiSnapshot::new();
        // Poll a unit that was never added
        let events = snapshot.poll_range(999, 0.0, 1.0);
        assert!(events.is_empty());
    }

    #[test]
    fn test_poll_skips_events_before_start() {
        let mut snapshot = MidiSnapshot::new();
        let unit_id = 1;

        // Add events at beats 0, 1, 2, 3
        snapshot.add_event(unit_id, 0.0, note_on(60, 100));
        snapshot.add_event(unit_id, 1.0, note_on(62, 100));
        snapshot.add_event(unit_id, 2.0, note_on(64, 100));
        snapshot.add_event(unit_id, 3.0, note_on(65, 100));

        // Poll starting at beat 2 - should skip beats 0 and 1
        let events = snapshot.poll_range(unit_id, 2.0, 4.0);
        assert_eq!(events.len(), 2); // Only beats 2 and 3
    }
}
