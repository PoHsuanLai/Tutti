//! RT-safe MIDI event registry for routing MIDI to audio graph nodes.
//!
//! Uses bounded SPSC channels per unit for lock-free poll on the audio thread.
//! The DashMap is only accessed via `get()` (read shard lock) — write locks
//! only occur during `register_unit()` which runs at setup time, never on
//! the audio thread.

use crate::compat::Arc;
use crossbeam_channel::{Receiver, Sender};
use dashmap::DashMap;

use tutti_midi::MidiEvent;

/// Maximum number of MIDI events buffered per unit per audio cycle.
///
/// 256 events covers extreme scenarios (full keyboard glissando + sustain + mod wheel).
/// If the channel is full, `queue()` drops events (producer back-pressure).
const EVENTS_PER_UNIT: usize = 256;

/// Per-unit MIDI event slot backed by a bounded SPSC channel.
struct MidiEventSlot {
    tx: Sender<MidiEvent>,
    rx: Receiver<MidiEvent>,
}

impl MidiEventSlot {
    fn new() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(EVENTS_PER_UNIT);
        Self { tx, rx }
    }
}

/// RT-safe registry for MIDI events destined for specific audio graph nodes.
///
/// Nodes poll this registry during their `process()` call to receive MIDI events.
/// The registry uses `AudioUnit::get_id()` as the lookup key.
///
/// # RT Safety
///
/// - `poll_into()`: Called on the audio thread. Uses `DashMap::get()`
///   (read shard lock — no contention with other readers) + `try_recv()` (lock-free).
///   No heap allocations. No blocking.
/// - `queue()`: Called from the UI/frontend thread. Uses `DashMap::get()` +
///   `try_send()` (lock-free). Falls back to `entry()` (write lock) only if
///   the unit was never registered — this should not happen on the audio thread path.
/// - `register_unit()`: Called at setup time to pre-create the channel.
#[derive(Clone)]
pub struct MidiRegistry {
    slots: Arc<DashMap<u64, Arc<MidiEventSlot>>>,
}

impl MidiRegistry {
    /// Create a new empty MIDI registry.
    pub fn new() -> Self {
        Self {
            slots: Arc::new(DashMap::new()),
        }
    }

    /// Pre-register a unit so its channel exists before audio processing starts.
    ///
    /// Call this at setup time (not on the audio thread). If the unit is already
    /// registered, this is a no-op.
    pub fn register_unit(&self, unit_id: u64) {
        self.slots
            .entry(unit_id)
            .or_insert_with(|| Arc::new(MidiEventSlot::new()));
    }

    /// Remove a unit's channel (cleanup on teardown).
    pub fn unregister_unit(&self, unit_id: u64) {
        self.slots.remove(&unit_id);
    }

    /// Queue MIDI events for a specific audio unit.
    ///
    /// Called from the UI/frontend thread. Events are available to `poll()` immediately.
    /// If the channel is full, excess events are silently dropped (back-pressure).
    ///
    /// If the unit has not been registered, it is auto-registered (involves a write
    /// lock on the DashMap shard — safe on the UI thread, but avoid calling this
    /// from the audio thread for unregistered units).
    pub fn queue(&self, unit_id: u64, events: &[MidiEvent]) {
        // Fast path: unit already registered (read lock only)
        if let Some(slot) = self.slots.get(&unit_id) {
            for &event in events {
                // try_send is lock-free; drops event if channel full
                let _ = slot.tx.try_send(event);
            }
            return;
        }

        // Slow path: auto-register (write lock — UI thread only)
        let slot = self
            .slots
            .entry(unit_id)
            .or_insert_with(|| Arc::new(MidiEventSlot::new()));
        for &event in events {
            let _ = slot.tx.try_send(event);
        }
    }

    /// Poll for MIDI events (RT-safe, returns count written to buffer).
    ///
    /// Drains all pending events into the provided buffer. Returns the number
    /// of events written. Zero allocations, no blocking.
    ///
    /// This is the preferred API for the audio thread.
    pub fn poll_into(&self, unit_id: u64, buffer: &mut [MidiEvent]) -> usize {
        let slot = match self.slots.get(&unit_id) {
            Some(s) => s,
            None => return 0,
        };

        let mut count = 0;
        for slot_ref in buffer.iter_mut() {
            match slot.rx.try_recv() {
                Ok(event) => {
                    *slot_ref = event;
                    count += 1;
                }
                Err(_) => break,
            }
        }
        count
    }

    /// Check if an audio unit has pending MIDI events.
    pub fn has_events(&self, unit_id: u64) -> bool {
        self.slots
            .get(&unit_id)
            .map(|slot| !slot.rx.is_empty())
            .unwrap_or(false)
    }

    /// Clear all pending MIDI events across all units.
    ///
    /// Called when resetting the audio graph.
    pub fn clear(&self) {
        for slot in self.slots.iter() {
            while slot.rx.try_recv().is_ok() {}
        }
    }

    /// Get the number of units with pending events.
    pub fn pending_count(&self) -> usize {
        self.slots.iter().filter(|slot| !slot.rx.is_empty()).count()
    }
}

impl Default for MidiRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compat::Vec;

    fn note_on(note: u8, vel: u8) -> MidiEvent {
        MidiEvent::note_on_builder(note, vel).build()
    }

    fn note_off(note: u8) -> MidiEvent {
        MidiEvent::note_off_builder(note).build()
    }

    #[test]
    fn test_queue_and_poll_into() {
        let registry = MidiRegistry::new();
        let unit_id = 12345u64;

        let events = vec![note_on(60, 100), note_off(60)];

        registry.queue(unit_id, &events);
        assert!(registry.has_events(unit_id));

        let mut buffer = [note_on(0, 0); 16];
        let count = registry.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 2);
        assert!(!registry.has_events(unit_id));
    }

    #[test]
    fn test_multiple_nodes() {
        let registry = MidiRegistry::new();
        let unit1 = 111u64;
        let unit2 = 222u64;

        registry.queue(unit1, &[note_on(60, 100)]);
        registry.queue(unit2, &[note_on(64, 100)]);

        assert_eq!(registry.pending_count(), 2);

        let mut buffer = [note_on(0, 0); 16];
        let count1 = registry.poll_into(unit1, &mut buffer);
        assert_eq!(count1, 1);
        assert_eq!(registry.pending_count(), 1);

        let count2 = registry.poll_into(unit2, &mut buffer);
        assert_eq!(count2, 1);
        assert_eq!(registry.pending_count(), 0);
    }

    #[test]
    fn test_poll_into_rt_safe() {
        let registry = MidiRegistry::new();
        let unit_id = 42u64;
        registry.register_unit(unit_id);

        registry.queue(
            unit_id,
            &[note_on(60, 100), note_on(64, 100), note_on(67, 100)],
        );

        // Pre-allocated buffer (simulating audio thread usage)
        let mut buffer = [note_on(0, 0); 16];
        let count = registry.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 3);
        assert!(!registry.has_events(unit_id));
    }

    #[test]
    fn test_register_unregister() {
        let registry = MidiRegistry::new();
        let unit_id = 99u64;

        registry.register_unit(unit_id);
        registry.queue(unit_id, &[note_on(60, 100)]);
        assert!(registry.has_events(unit_id));

        registry.unregister_unit(unit_id);
        // After unregister, poll_into returns 0
        let mut buffer = [note_on(0, 0); 16];
        let count = registry.poll_into(unit_id, &mut buffer);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_backpressure() {
        let registry = MidiRegistry::new();
        let unit_id = 1u64;
        registry.register_unit(unit_id);

        // Flood with more events than channel capacity
        let events: Vec<_> = (0..512).map(|i| note_on((i % 128) as u8, 100)).collect();
        registry.queue(unit_id, &events);

        // Should get at most EVENTS_PER_UNIT events
        let mut buffer = [note_on(0, 0); 512];
        let count = registry.poll_into(unit_id, &mut buffer);
        assert!(count <= super::EVENTS_PER_UNIT);
        assert!(count > 0);
    }
}
