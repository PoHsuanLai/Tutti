//! Global MIDI event registry for routing MIDI to nodes
//!
//! This provides a lock-free way to send MIDI events to nodes in the audio graph
//! without requiring trait downcasting or complex lifetime management.

use dashmap::DashMap;
use std::sync::Arc;

use crate::midi::MidiEvent;

/// Thread-safe registry for MIDI events destined for specific nodes.
///
/// Nodes poll this registry during their `process()` call to receive MIDI events.
/// The registry uses AudioUnit::get_id() as the lookup key, so nodes don't need
/// to know their NodeId.
#[derive(Clone)]
pub struct MidiRegistry {
    /// Map of AudioUnit ID -> pending MIDI events
    events: Arc<DashMap<u64, Vec<MidiEvent>>>,
}

impl MidiRegistry {
    /// Create a new empty MIDI registry
    pub fn new() -> Self {
        Self {
            events: Arc::new(DashMap::new()),
        }
    }

    /// Queue MIDI events for a specific audio unit
    ///
    /// Events will be available for the node to poll in the next audio cycle.
    ///
    /// # Arguments
    /// * `unit_id` - The AudioUnit::get_id() value to send events to
    /// * `events` - Slice of MIDI events to queue
    pub fn queue(&self, unit_id: u64, events: &[MidiEvent]) {
        self.events
            .entry(unit_id)
            .or_insert_with(Vec::new)
            .extend_from_slice(events);
    }

    /// Poll for MIDI events for a specific audio unit
    ///
    /// This should be called from the node's `process()` method.
    /// Returns all pending events for this node and clears them.
    ///
    /// # Arguments
    /// * `unit_id` - The AudioUnit::get_id() value to poll events for
    ///
    /// # Returns
    /// Vector of MIDI events, or empty if none pending
    pub fn poll(&self, unit_id: u64) -> Vec<MidiEvent> {
        self.events
            .remove(&unit_id)
            .map(|(_, events)| events)
            .unwrap_or_default()
    }

    /// Check if an audio unit has pending MIDI events
    pub fn has_events(&self, unit_id: u64) -> bool {
        self.events.contains_key(&unit_id)
    }

    /// Clear all pending MIDI events
    ///
    /// This is called when resetting the audio graph.
    pub fn clear(&self) {
        self.events.clear();
    }

    /// Get the number of nodes with pending events
    pub fn pending_count(&self) -> usize {
        self.events.len()
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

    #[test]
    fn test_queue_and_poll() {
        let registry = MidiRegistry::new();
        let unit_id = 12345u64;

        let events = vec![
            MidiEvent::note_on(0, 60, 100, 0),
            MidiEvent::note_off(0, 60, 0, 480),
        ];

        registry.queue(unit_id, &events);
        assert!(registry.has_events(unit_id));

        let polled = registry.poll(unit_id);
        assert_eq!(polled.len(), 2);
        assert!(!registry.has_events(unit_id));
    }

    #[test]
    fn test_multiple_nodes() {
        let registry = MidiRegistry::new();
        let unit1 = 111u64;
        let unit2 = 222u64;

        registry.queue(unit1, &[MidiEvent::note_on(0, 60, 100, 0)]);
        registry.queue(unit2, &[MidiEvent::note_on(0, 64, 100, 0)]);

        assert_eq!(registry.pending_count(), 2);

        let events1 = registry.poll(unit1);
        assert_eq!(events1.len(), 1);
        assert_eq!(registry.pending_count(), 1);

        let events2 = registry.poll(unit2);
        assert_eq!(events2.len(), 1);
        assert_eq!(registry.pending_count(), 0);
    }
}
