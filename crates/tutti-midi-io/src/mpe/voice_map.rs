//! MPE channel-voice mapping for MIDI 1.0 MPE.

use super::zone::MpeZoneConfig;

/// Maps member channels to active notes for MIDI 1.0 MPE
///
/// In MPE, each note gets its own channel. This struct tracks which
/// channel is currently playing which note, allowing proper routing
/// of per-channel expression to per-note expression.
#[derive(Debug)]
pub(crate) struct MpeChannelVoiceMap {
    /// Channel (0-15) → Note number (or None if unused)
    pub(crate) channel_to_note: [Option<u8>; 16],
    /// Note number → Channel (or None if not playing)
    pub(crate) note_to_channel: [Option<u8>; 128],
    /// Round-robin index for channel allocation
    next_channel_index: usize,
    /// Zone configuration for channel validation
    zone_config: MpeZoneConfig,
}

impl MpeChannelVoiceMap {
    /// Create a new channel-voice map for a zone
    pub(crate) fn new(zone_config: MpeZoneConfig) -> Self {
        Self {
            channel_to_note: [None; 16],
            note_to_channel: [None; 128],
            next_channel_index: 0,
            zone_config,
        }
    }

    /// Assign a channel to a note (on Note On)
    ///
    /// Returns the assigned channel, or None if no channels available.
    pub fn assign_note(&mut self, note: u8) -> Option<u8> {
        if note >= 128 {
            return None;
        }

        // If note is already assigned, return its channel
        if let Some(ch) = self.note_to_channel[note as usize] {
            return Some(ch);
        }

        // Find a free member channel using round-robin
        let member_range = self.zone_config.member_channel_range();
        let member_count = *member_range.end() - *member_range.start() + 1;

        for offset in 0..member_count {
            let index = (self.next_channel_index + offset as usize) % member_count as usize;
            let channel = *member_range.start() + index as u8;

            if self.channel_to_note[channel as usize].is_none() {
                // Found a free channel
                self.channel_to_note[channel as usize] = Some(note);
                self.note_to_channel[note as usize] = Some(channel);
                self.next_channel_index = (index + 1) % member_count as usize;
                return Some(channel);
            }
        }

        // No free channels - voice stealing would go here
        // For now, reuse the oldest channel (round-robin)
        let channel = *member_range.start() + self.next_channel_index as u8;
        if let Some(old_note) = self.channel_to_note[channel as usize] {
            self.note_to_channel[old_note as usize] = None;
        }
        self.channel_to_note[channel as usize] = Some(note);
        self.note_to_channel[note as usize] = Some(channel);
        self.next_channel_index = (self.next_channel_index + 1) % member_count as usize;
        Some(channel)
    }

    /// Release a note (on Note Off)
    pub fn release_note(&mut self, note: u8) {
        if note >= 128 {
            return;
        }

        if let Some(channel) = self.note_to_channel[note as usize] {
            self.channel_to_note[channel as usize] = None;
            self.note_to_channel[note as usize] = None;
        }
    }

    /// Get the note playing on a channel
    #[inline]
    pub fn get_note_for_channel(&self, channel: u8) -> Option<u8> {
        if channel < 16 {
            self.channel_to_note[channel as usize]
        } else {
            None
        }
    }

    /// Get the channel assigned to a note
    #[inline]
    pub fn get_channel_for_note(&self, note: u8) -> Option<u8> {
        if note < 128 {
            self.note_to_channel[note as usize]
        } else {
            None
        }
    }

    /// Check if this map handles the given channel
    #[inline]
    pub fn handles_channel(&self, channel: u8) -> bool {
        self.zone_config.handles_channel(channel)
    }

    /// Clear all mappings
    pub fn clear(&mut self) {
        self.channel_to_note = [None; 16];
        self.note_to_channel = [None; 128];
        self.next_channel_index = 0;
    }
}

/// Zone information for a channel
#[derive(Clone, Copy, Debug)]
pub(crate) struct ZoneInfo {
    pub(crate) is_master: bool,
    pub(crate) is_member: bool,
    pub(crate) is_lower_zone: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_voice_map() {
        let config = MpeZoneConfig::lower(3);
        let mut map = MpeChannelVoiceMap::new(config);

        // Assign note 60
        let ch1 = map.assign_note(60);
        assert!(ch1.is_some());
        assert!(map.get_channel_for_note(60).is_some());

        // Assign note 62
        let ch2 = map.assign_note(62);
        assert!(ch2.is_some());
        assert_ne!(ch1, ch2);

        // Release note 60
        map.release_note(60);
        assert!(map.get_channel_for_note(60).is_none());

        // Note 62 should still be assigned
        assert!(map.get_channel_for_note(62).is_some());
    }
}
