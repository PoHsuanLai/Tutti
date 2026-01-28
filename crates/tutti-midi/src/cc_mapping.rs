//! MIDI CC (Control Change) mapping system
//!
//! Allows MIDI controllers to control synth/effect parameters in real-time

use std::collections::HashMap;

/// MIDI CC number (0-127)
pub type CCNumber = u8;

/// MIDI channel (0-15, where 0 = channel 1)
pub type MidiChannel = u8;

/// Target for a MIDI CC mapping
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CCTarget {
    /// Track volume (index-based)
    TrackVolume(usize),
    /// Track pan (index-based)
    TrackPan(usize),
    /// Track effect parameter (index-based)
    EffectParam {
        track_index: usize,
        effect_slot: u8,  // Effect slot in the chain (0-255)
        param_index: u16, // Parameter index (0-65535)
    },
    /// Master volume
    MasterVolume,
    /// Tempo
    Tempo,
}

/// A MIDI CC mapping entry
#[derive(Debug, Clone)]
pub struct CCMapping {
    /// MIDI channel (None = all channels)
    pub channel: Option<MidiChannel>,
    /// CC number
    pub cc_number: CCNumber,
    /// Target parameter
    pub target: CCTarget,
    /// Minimum value (maps CC 0 to this value)
    pub min_value: f32,
    /// Maximum value (maps CC 127 to this value)
    pub max_value: f32,
    /// Is this mapping enabled?
    pub enabled: bool,
}

impl CCMapping {
    /// Create a new CC mapping
    pub fn new(
        channel: Option<MidiChannel>,
        cc_number: CCNumber,
        target: CCTarget,
        min_value: f32,
        max_value: f32,
    ) -> Self {
        Self {
            channel,
            cc_number,
            target,
            min_value,
            max_value,
            enabled: true,
        }
    }

    /// Map a CC value (0-127) to the parameter range
    pub fn map_value(&self, cc_value: u8) -> f32 {
        let normalized = cc_value as f32 / 127.0;
        self.min_value + normalized * (self.max_value - self.min_value)
    }

    /// Check if this mapping matches the given channel and CC number
    pub fn matches(&self, channel: MidiChannel, cc_number: CCNumber) -> bool {
        if !self.enabled {
            return false;
        }
        let channel_matches = self.channel.is_none() || self.channel == Some(channel);
        channel_matches && self.cc_number == cc_number
    }
}

/// Unique ID for a CC mapping
pub type MappingId = u64;

/// MIDI CC mapping registry
pub struct CCMappingRegistry {
    /// All registered mappings
    mappings: HashMap<MappingId, CCMapping>,
    /// Next mapping ID
    next_id: MappingId,
    /// MIDI learn mode state
    learn_state: Option<LearnState>,
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

impl CCMappingRegistry {
    /// Create a new CC mapping registry
    pub fn new() -> Self {
        Self {
            mappings: HashMap::new(),
            next_id: 0,
            learn_state: None,
        }
    }

    /// Add a new CC mapping
    pub fn add_mapping(&mut self, mapping: CCMapping) -> MappingId {
        let id = self.next_id;
        self.next_id += 1;
        self.mappings.insert(id, mapping);
        id
    }

    /// Remove a CC mapping
    pub fn remove_mapping(&mut self, id: MappingId) -> bool {
        self.mappings.remove(&id).is_some()
    }

    /// Get a mapping by ID
    pub fn get_mapping(&self, id: MappingId) -> Option<&CCMapping> {
        self.mappings.get(&id)
    }

    /// Get a mutable mapping by ID
    pub fn get_mapping_mut(&mut self, id: MappingId) -> Option<&mut CCMapping> {
        self.mappings.get_mut(&id)
    }

    /// Get all mappings
    pub fn get_all_mappings(&self) -> Vec<(MappingId, &CCMapping)> {
        self.mappings
            .iter()
            .map(|(id, mapping)| (*id, mapping))
            .collect()
    }

    /// Find mappings that match a given channel and CC number
    pub fn find_mappings(
        &self,
        channel: MidiChannel,
        cc_number: CCNumber,
    ) -> Vec<(MappingId, &CCMapping)> {
        self.mappings
            .iter()
            .filter(|(_, mapping)| mapping.matches(channel, cc_number))
            .map(|(id, mapping)| (*id, mapping))
            .collect()
    }

    /// Enable/disable a mapping
    pub fn set_mapping_enabled(&mut self, id: MappingId, enabled: bool) -> bool {
        if let Some(mapping) = self.mappings.get_mut(&id) {
            mapping.enabled = enabled;
            true
        } else {
            false
        }
    }

    /// Clear all mappings
    pub fn clear_all(&mut self) {
        self.mappings.clear();
    }

    /// Start MIDI learn mode for a target
    pub fn start_learn(
        &mut self,
        target: CCTarget,
        min_value: f32,
        max_value: f32,
        channel_filter: Option<MidiChannel>,
    ) {
        self.learn_state = Some(LearnState {
            target,
            min_value,
            max_value,
            channel_filter,
        });
    }

    /// Cancel MIDI learn mode
    pub fn cancel_learn(&mut self) {
        self.learn_state = None;
    }

    /// Check if we're in learn mode
    pub fn is_learning(&self) -> bool {
        self.learn_state.is_some()
    }

    /// Process a CC message in learn mode
    /// Returns the new mapping ID if learning was successful
    pub fn process_learn(
        &mut self,
        channel: MidiChannel,
        cc_number: CCNumber,
    ) -> Option<MappingId> {
        if let Some(learn_state) = self.learn_state.take() {
            if let Some(filter_channel) = learn_state.channel_filter {
                if channel != filter_channel {
                    // Wrong channel, put learn state back
                    self.learn_state = Some(learn_state);
                    return None;
                }
            }

            let mapping = CCMapping::new(
                Some(channel),
                cc_number,
                learn_state.target,
                learn_state.min_value,
                learn_state.max_value,
            );

            Some(self.add_mapping(mapping))
        } else {
            None
        }
    }

    /// Get the current learn state target (if learning)
    pub fn get_learn_target(&self) -> Option<&CCTarget> {
        self.learn_state.as_ref().map(|state| &state.target)
    }
}

impl Default for CCMappingRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cc_mapping_creation() {
        let mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        assert_eq!(mapping.channel, Some(0));
        assert_eq!(mapping.cc_number, 1);
        assert_eq!(mapping.min_value, 0.0);
        assert_eq!(mapping.max_value, 1.0);
        assert!(mapping.enabled);
    }

    #[test]
    fn test_map_value() {
        let mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        assert_eq!(mapping.map_value(0), 0.0);
        assert_eq!(mapping.map_value(127), 1.0);
        assert!((mapping.map_value(64) - 0.504).abs() < 0.01); // Approximately 0.5
    }

    #[test]
    fn test_matches() {
        let mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        assert!(mapping.matches(0, 1));
        assert!(!mapping.matches(1, 1)); // Wrong channel
        assert!(!mapping.matches(0, 2)); // Wrong CC

        // Test any channel
        let any_channel = CCMapping::new(None, 1, CCTarget::MasterVolume, 0.0, 1.0);
        assert!(any_channel.matches(0, 1));
        assert!(any_channel.matches(15, 1));
    }

    #[test]
    fn test_registry() {
        let mut registry = CCMappingRegistry::new();
        let mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        let id = registry.add_mapping(mapping);
        assert_eq!(id, 0);
        assert_eq!(registry.mappings.len(), 1);
    }

    #[test]
    fn test_find_mappings() {
        let mut registry = CCMappingRegistry::new();
        registry.add_mapping(CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0));
        registry.add_mapping(CCMapping::new(Some(0), 2, CCTarget::Tempo, 60.0, 200.0));

        let found = registry.find_mappings(0, 1);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1.target, CCTarget::MasterVolume);
    }

    #[test]
    fn test_learn_mode() {
        let mut registry = CCMappingRegistry::new();
        assert!(!registry.is_learning());

        registry.start_learn(CCTarget::MasterVolume, 0.0, 1.0, None);
        assert!(registry.is_learning());

        let id = registry.process_learn(0, 7);
        assert!(id.is_some());
        assert!(!registry.is_learning());

        let mapping = registry.get_mapping(id.unwrap()).unwrap();
        assert_eq!(mapping.cc_number, 7);
        assert_eq!(mapping.channel, Some(0));
    }

    #[test]
    fn test_learn_channel_filter() {
        let mut registry = CCMappingRegistry::new();
        registry.start_learn(CCTarget::MasterVolume, 0.0, 1.0, Some(1));

        // Wrong channel - should stay in learn mode
        let id = registry.process_learn(0, 7);
        assert!(id.is_none());
        assert!(registry.is_learning());

        // Correct channel - should complete
        let id = registry.process_learn(1, 7);
        assert!(id.is_some());
        assert!(!registry.is_learning());
    }

    #[test]
    fn test_enable_disable() {
        let mut registry = CCMappingRegistry::new();
        let mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        let id = registry.add_mapping(mapping);

        registry.set_mapping_enabled(id, false);
        let found = registry.find_mappings(0, 1);
        assert_eq!(found.len(), 0); // Disabled mapping should not be found
    }
}
