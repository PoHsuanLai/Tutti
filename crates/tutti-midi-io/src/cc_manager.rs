//! MIDI CC mapping manager with MIDI learn support.

use crate::cc_mapping::{CCMapping, CCNumber, CCTarget, MappingId, MidiChannel};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Result of processing a CC message
#[derive(Debug, Clone)]
pub struct CCProcessResult {
    /// Target and mapped value pairs to apply
    pub targets: Vec<(CCTarget, f32)>,
    /// Whether MIDI learn was completed (and the new mapping ID)
    pub learn_completed: Option<MappingId>,
}

/// MIDI learn mode state
#[derive(Debug, Clone)]
struct LearnState {
    /// The target we're learning for
    target: CCTarget,
    /// Min/max values for the mapping
    min_value: f32,
    max_value: f32,
    /// Channel filter (None = any channel)
    channel_filter: Option<MidiChannel>,
}

/// Lock-free MIDI CC mapping manager with MIDI learn mode.
pub struct CCMappingManager {
    mappings: Arc<DashMap<MappingId, CCMapping>>,
    next_id: AtomicU64,
    learn_state: ArcSwap<Option<LearnState>>,
}

impl CCMappingManager {
    /// Create a new CC mapping manager
    pub fn new() -> Self {
        Self {
            mappings: Arc::new(DashMap::new()),
            next_id: AtomicU64::new(1),
            learn_state: ArcSwap::new(Arc::new(None)),
        }
    }

    /// Get shared access to the mappings
    pub fn mappings_arc(&self) -> Arc<DashMap<MappingId, CCMapping>> {
        Arc::clone(&self.mappings)
    }

    // ==================== Mapping CRUD ====================

    /// Add a MIDI CC mapping
    pub fn add_mapping(
        &self,
        channel: Option<MidiChannel>,
        cc_number: CCNumber,
        target: CCTarget,
        min_value: f32,
        max_value: f32,
    ) -> MappingId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mapping = CCMapping::new(channel, cc_number, target, min_value, max_value);
        self.mappings.insert(id, mapping);
        id
    }

    /// Remove a MIDI CC mapping
    pub fn remove_mapping(&self, mapping_id: MappingId) -> bool {
        self.mappings.remove(&mapping_id).is_some()
    }

    /// Get all MIDI CC mappings
    pub fn get_all_mappings(&self) -> Vec<(MappingId, CCMapping)> {
        self.mappings
            .iter()
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }

    /// Get a specific MIDI CC mapping by ID
    pub fn get_mapping(&self, mapping_id: MappingId) -> Option<CCMapping> {
        self.mappings
            .get(&mapping_id)
            .map(|entry| entry.value().clone())
    }

    /// Find all mappings for a specific channel and CC number
    pub fn find_mappings(
        &self,
        channel: MidiChannel,
        cc_number: CCNumber,
    ) -> Vec<(MappingId, CCMapping)> {
        self.mappings
            .iter()
            .filter(|entry| {
                let mapping = entry.value();
                mapping.matches(channel, cc_number)
            })
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }

    /// Enable/disable a MIDI CC mapping
    pub fn set_mapping_enabled(&self, mapping_id: MappingId, enabled: bool) -> bool {
        if let Some(mut entry) = self.mappings.get_mut(&mapping_id) {
            entry.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Clear all MIDI CC mappings
    pub fn clear_all(&self) {
        self.mappings.clear();
    }

    // ==================== MIDI Learn ====================

    /// Start MIDI learn mode for a target
    pub fn start_learn(
        &self,
        target: CCTarget,
        min_value: f32,
        max_value: f32,
        channel_filter: Option<MidiChannel>,
    ) {
        let state = LearnState {
            target,
            min_value,
            max_value,
            channel_filter,
        };
        self.learn_state.store(Arc::new(Some(state)));
    }

    /// Cancel MIDI learn mode
    pub fn cancel_learn(&self) {
        self.learn_state.store(Arc::new(None));
    }

    /// Check if we're in MIDI learn mode
    pub fn is_learning(&self) -> bool {
        self.learn_state.load().is_some()
    }

    /// Get the current MIDI learn target (if learning)
    pub fn get_learn_target(&self) -> Option<CCTarget> {
        let guard = self.learn_state.load();
        guard.as_ref().as_ref().map(|state| state.target.clone())
    }

    // ==================== CC Processing ====================

    /// Process a MIDI CC message and return targets to apply (lock-free!)
    ///
    /// This handles MIDI learn mode and returns the mapped targets.
    /// The caller is responsible for actually applying the values to
    /// track controls, effects, transport, etc.
    ///
    /// Returns `CCProcessResult` with targets to apply and learn status.
    ///
    /// **Lock-free**: Uses ArcSwap for learn state, DashMap for mappings.
    pub fn process_cc(
        &self,
        channel: MidiChannel,
        cc_number: CCNumber,
        cc_value: u8,
    ) -> CCProcessResult {
        // Check if we're in learn mode
        let learn_guard = self.learn_state.load();
        if let Some(ref state) = **learn_guard {
            if state.channel_filter.is_none() || state.channel_filter == Some(channel) {
                // Complete the learn - create new mapping
                let mapping_id = self.add_mapping(
                    Some(channel),
                    cc_number,
                    state.target.clone(),
                    state.min_value,
                    state.max_value,
                );

                // Exit learn mode
                self.cancel_learn();

                return CCProcessResult {
                    targets: vec![],
                    learn_completed: Some(mapping_id),
                };
            }
            // Still learning, waiting for right channel
            return CCProcessResult {
                targets: vec![],
                learn_completed: None,
            };
        }

        // Find and collect matching mappings (lock-free iteration over DashMap)
        let targets: Vec<(CCTarget, f32)> = self
            .mappings
            .iter()
            .filter(|entry| {
                let mapping = entry.value();
                mapping.enabled
                    && mapping.cc_number == cc_number
                    && (mapping.channel.is_none() || mapping.channel == Some(channel))
            })
            .map(|entry| {
                let mapping = entry.value();
                let value = mapping.map_value(cc_value);
                (mapping.target.clone(), value)
            })
            .collect();

        CCProcessResult {
            targets,
            learn_completed: None,
        }
    }
}

impl Default for CCMappingManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manager_add_remove() {
        let manager = CCMappingManager::new();
        let id = manager.add_mapping(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);

        let mappings = manager.get_all_mappings();
        assert_eq!(mappings.len(), 1);

        assert!(manager.remove_mapping(id));
        assert_eq!(manager.get_all_mappings().len(), 0);
    }

    #[test]
    fn test_manager_process_cc() {
        let manager = CCMappingManager::new();
        manager.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

        let result = manager.process_cc(0, 7, 127);
        assert_eq!(result.targets.len(), 1);
        assert_eq!(result.targets[0].0, CCTarget::MasterVolume);
        assert!((result.targets[0].1 - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_manager_learn_mode() {
        let manager = CCMappingManager::new();

        manager.start_learn(CCTarget::Tempo, 60.0, 200.0, None);
        assert!(manager.is_learning());

        let result = manager.process_cc(0, 11, 64);
        assert!(result.learn_completed.is_some());
        assert!(!manager.is_learning());

        // Now the mapping should work
        let result = manager.process_cc(0, 11, 127);
        assert_eq!(result.targets.len(), 1);
        assert_eq!(result.targets[0].0, CCTarget::Tempo);
    }
}
