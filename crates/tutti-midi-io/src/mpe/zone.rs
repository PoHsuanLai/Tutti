//! MPE Zone configuration types.

/// MPE Zone type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpeZone {
    /// Lower Zone
    Lower,
    /// Upper Zone
    Upper,
    /// Single channel mode
    SingleChannel(u8),
}

/// Zone configuration for MPE
#[derive(Clone, Debug)]
pub struct MpeZoneConfig {
    pub zone: MpeZone,
    pub master_channel: u8,
    pub member_count: u8,
    pub pitch_bend_range: u8,
    pub enabled: bool,
}

impl MpeZoneConfig {
    /// Create a Lower Zone configuration
    pub fn lower(member_count: u8) -> Self {
        let member_count = member_count.clamp(1, 15);
        Self {
            zone: MpeZone::Lower,
            master_channel: 0, // Ch1 (0-indexed)
            member_count,
            pitch_bend_range: 48,
            enabled: true,
        }
    }

    /// Create an Upper Zone configuration
    pub fn upper(member_count: u8) -> Self {
        let member_count = member_count.clamp(1, 15);
        Self {
            zone: MpeZone::Upper,
            master_channel: 15, // Ch16 (0-indexed)
            member_count,
            pitch_bend_range: 48,
            enabled: true,
        }
    }

    /// Create a single channel configuration
    pub fn single_channel(channel: u8) -> Self {
        Self {
            zone: MpeZone::SingleChannel(channel.min(15)),
            master_channel: channel.min(15),
            member_count: 0,
            pitch_bend_range: 2, // Standard non-MPE default
            enabled: true,
        }
    }

    /// Set the pitch bend range in semitones
    pub fn with_pitch_bend_range(mut self, semitones: u8) -> Self {
        self.pitch_bend_range = semitones;
        self
    }

    /// Check if a channel is the master channel
    #[inline]
    pub fn is_master_channel(&self, channel: u8) -> bool {
        channel == self.master_channel
    }

    /// Check if a channel is a member channel
    #[inline]
    pub fn is_member_channel(&self, channel: u8) -> bool {
        match self.zone {
            MpeZone::Lower => {
                // Members: Ch2 (1) to Ch(1+member_count)
                channel >= 1 && channel <= self.member_count
            }
            MpeZone::Upper => {
                // Members: Ch15 (14) down to Ch(16-member_count)
                let lowest_member = 15 - self.member_count;
                channel >= lowest_member && channel <= 14
            }
            MpeZone::SingleChannel(_) => false,
        }
    }

    /// Check if this zone handles the given channel
    #[inline]
    pub fn handles_channel(&self, channel: u8) -> bool {
        self.is_master_channel(channel) || self.is_member_channel(channel)
    }

    /// Get member channels as a range
    pub fn member_channel_range(&self) -> std::ops::RangeInclusive<u8> {
        match self.zone {
            MpeZone::Lower => 1..=self.member_count,
            MpeZone::Upper => (15 - self.member_count)..=14,
            MpeZone::SingleChannel(ch) => ch..=ch,
        }
    }
}

/// MPE mode configuration
#[derive(Clone, Debug, Default)]
pub enum MpeMode {
    #[default]
    Disabled,
    LowerZone(MpeZoneConfig),
    UpperZone(MpeZoneConfig),
    DualZone {
        lower: MpeZoneConfig,
        upper: MpeZoneConfig,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_config_lower() {
        let config = MpeZoneConfig::lower(10);
        assert_eq!(config.master_channel, 0);
        assert_eq!(config.member_count, 10);
        assert!(config.is_master_channel(0));
        assert!(!config.is_master_channel(1));
        assert!(config.is_member_channel(1));
        assert!(config.is_member_channel(10));
        assert!(!config.is_member_channel(11));
        assert!(!config.is_member_channel(0));
    }

    #[test]
    fn test_zone_config_upper() {
        let config = MpeZoneConfig::upper(5);
        assert_eq!(config.master_channel, 15);
        assert_eq!(config.member_count, 5);
        assert!(config.is_master_channel(15));
        assert!(!config.is_master_channel(14));
        // Upper zone: members are 15-member_count to 14 (i.e., 10 to 14)
        assert!(config.is_member_channel(14));
        assert!(config.is_member_channel(10));
        assert!(!config.is_member_channel(9));
        assert!(!config.is_member_channel(15));
    }
}
