//! MIDI CC (Control Change) mapping types.
//!
//! For the thread-safe mapping manager, see `CCMappingManager` in `cc_manager`.

/// MIDI CC number (0-127)
pub type CCNumber = u8;

/// MIDI channel (0-15, where 0 = channel 1)
pub type MidiChannel = u8;

/// Unique ID for a CC mapping
pub type MappingId = u64;

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
    #[inline]
    pub fn map_value(&self, cc_value: u8) -> f32 {
        let normalized = cc_value as f32 / 127.0;
        self.min_value + normalized * (self.max_value - self.min_value)
    }

    /// Check if this mapping matches the given channel and CC number
    #[inline]
    pub fn matches(&self, channel: MidiChannel, cc_number: CCNumber) -> bool {
        if !self.enabled {
            return false;
        }
        let channel_matches = self.channel.is_none() || self.channel == Some(channel);
        channel_matches && self.cc_number == cc_number
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
        assert!((mapping.map_value(64) - 0.504).abs() < 0.01);
    }

    #[test]
    fn test_map_value_custom_range() {
        let mapping = CCMapping::new(Some(0), 1, CCTarget::Tempo, 60.0, 200.0);
        assert_eq!(mapping.map_value(0), 60.0);
        assert_eq!(mapping.map_value(127), 200.0);
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
    fn test_matches_disabled() {
        let mut mapping = CCMapping::new(Some(0), 1, CCTarget::MasterVolume, 0.0, 1.0);
        assert!(mapping.matches(0, 1));

        mapping.enabled = false;
        assert!(!mapping.matches(0, 1));
    }
}
