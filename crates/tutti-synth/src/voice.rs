//! Polyphonic voice allocator with multiple stealing strategies.
//!
//! Manages voice allocation for polyphonic synthesizers:
//! - Stealing strategies: oldest, quietest, highest/lowest note
//! - Mono and legato modes
//! - Sustain and sostenuto pedal handling
//!
//! All methods are RT-safe (no allocations after construction).

/// Unique identifier for a voice instance.
pub type VoiceId = u64;

/// Voice allocation strategy when no idle voices are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocationStrategy {
    /// Steal the oldest playing voice (most common)
    #[default]
    Oldest,
    /// Steal the quietest voice (lowest envelope level)
    Quietest,
    /// Steal the highest pitched voice
    HighestNote,
    /// Steal the lowest pitched voice
    LowestNote,
    /// Steal the most recent voice
    Newest,
    /// Never steal - drop new notes if no voices available
    NoSteal,
}

/// Voice playing mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceMode {
    /// Polyphonic - multiple simultaneous voices
    #[default]
    Poly,
    /// Monophonic - one voice, always retrigger envelope
    Mono,
    /// Legato - one voice, glide between notes without retriggering
    Legato,
}

/// State of a single voice slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceState {
    /// Voice is idle and available for allocation
    #[default]
    Idle,
    /// Voice is actively playing (attack/decay/sustain)
    Active,
    /// Voice is in release phase
    Releasing,
    /// Voice was stolen but still fading out
    Stolen,
}

/// Information about an allocated voice slot.
#[derive(Debug, Clone)]
pub struct VoiceSlot {
    /// Unique voice identifier
    pub voice_id: VoiceId,
    /// MIDI note number that triggered this voice (0-127)
    pub note: u8,
    /// MIDI channel (0-15)
    pub channel: u8,
    /// Velocity (0.0-1.0, normalized)
    pub velocity: f32,
    /// Current envelope level (0.0-1.0) for quietest stealing
    pub envelope_level: f32,
    /// When this voice started (sample count)
    pub start_time: u64,
    /// Current voice state
    pub state: VoiceState,
    /// True if sustain pedal is holding this voice
    pub sustained: bool,
    /// True if sostenuto pedal is holding this voice
    pub sostenuto_held: bool,
}

impl Default for VoiceSlot {
    fn default() -> Self {
        Self {
            voice_id: 0,
            note: 0,
            channel: 0,
            velocity: 0.0,
            envelope_level: 0.0,
            start_time: 0,
            state: VoiceState::Idle,
            sustained: false,
            sostenuto_held: false,
        }
    }
}

/// Configuration for the voice allocator.
#[derive(Debug, Clone)]
pub struct VoiceAllocatorConfig {
    /// Maximum number of simultaneous voices
    pub max_voices: usize,
    /// Strategy for stealing voices when none are idle
    pub strategy: AllocationStrategy,
    /// Voice playing mode (poly/mono/legato)
    pub mode: VoiceMode,
    /// Time in samples to crossfade when stealing (default: 64 samples ~1.5ms)
    pub steal_crossfade_samples: usize,
}

impl Default for VoiceAllocatorConfig {
    fn default() -> Self {
        Self {
            max_voices: 16,
            strategy: AllocationStrategy::Oldest,
            mode: VoiceMode::Poly,
            steal_crossfade_samples: 64,
        }
    }
}

/// Result of attempting to allocate a voice.
#[derive(Debug, Clone)]
pub enum AllocationResult {
    /// New voice successfully allocated
    Allocated {
        voice_id: VoiceId,
        slot_index: usize,
    },
    /// Existing voice was stolen to make room
    Stolen {
        voice_id: VoiceId,
        slot_index: usize,
        stolen_voice_id: VoiceId,
    },
    /// Legato transition - update existing voice without retriggering
    LegatoRetrigger {
        voice_id: VoiceId,
        slot_index: usize,
    },
    /// No voice available (NoSteal strategy or error)
    Unavailable,
}

/// Polyphonic voice allocator.
///
/// Manages voice allocation, stealing, and pedal handling for synthesizers.
/// All methods are RT-safe (no allocations after construction).
pub struct VoiceAllocator {
    config: VoiceAllocatorConfig,
    slots: Vec<VoiceSlot>,
    next_voice_id: VoiceId,
    current_time: u64,

    /// Active note tracking: note (0-127) -> slot index
    /// Fixed-size array avoids HashMap allocations
    note_to_slot: [Option<usize>; 128],

    /// Sustain pedal state per channel (0-15)
    sustain_pedal: [bool; 16],

    /// Sostenuto pedal state per channel (0-15)
    sostenuto_pedal: [bool; 16],

    /// Last note for legato mode
    legato_last_note: Option<u8>,
}

impl VoiceAllocator {
    /// Create a new voice allocator with the given configuration.
    pub fn new(config: VoiceAllocatorConfig) -> Self {
        let slots = (0..config.max_voices)
            .map(|_| VoiceSlot::default())
            .collect();

        Self {
            config,
            slots,
            next_voice_id: 1,
            current_time: 0,
            note_to_slot: [None; 128],
            sustain_pedal: [false; 16],
            sostenuto_pedal: [false; 16],
            legato_last_note: None,
        }
    }

    /// Allocate a voice for a new note.
    ///
    /// RT-safe: no heap allocations.
    ///
    /// # Arguments
    /// * `note` - MIDI note number (0-127)
    /// * `channel` - MIDI channel (0-15)
    /// * `velocity` - Note velocity (0.0-1.0)
    pub fn allocate(&mut self, note: u8, channel: u8, velocity: f32) -> AllocationResult {
        // Mono/Legato mode handling
        if self.config.mode != VoiceMode::Poly {
            return self.allocate_mono_legato(note, channel, velocity);
        }

        // If this note is already playing, release it first
        if let Some(existing_slot) = self.note_to_slot[note as usize] {
            self.slots[existing_slot].state = VoiceState::Releasing;
            self.note_to_slot[note as usize] = None;
        }

        // Find an idle slot
        if let Some(slot_index) = self.find_idle_slot() {
            return self.activate_slot(slot_index, note, channel, velocity);
        }

        // Find a slot to steal based on strategy
        if self.config.strategy == AllocationStrategy::NoSteal {
            return AllocationResult::Unavailable;
        }

        if let Some(slot_index) = self.find_slot_to_steal() {
            let stolen_voice_id = self.slots[slot_index].voice_id;
            let old_note = self.slots[slot_index].note;

            // Clear old note mapping
            self.note_to_slot[old_note as usize] = None;

            // Mark as stolen (caller should crossfade)
            self.slots[slot_index].state = VoiceState::Stolen;

            // Activate new voice in this slot
            let voice_id = self.next_voice_id;
            self.next_voice_id += 1;

            self.slots[slot_index] = VoiceSlot {
                voice_id,
                note,
                channel,
                velocity,
                envelope_level: 0.0,
                start_time: self.current_time,
                state: VoiceState::Active,
                sustained: false,
                sostenuto_held: self.sostenuto_pedal[channel as usize],
            };

            self.note_to_slot[note as usize] = Some(slot_index);

            AllocationResult::Stolen {
                voice_id,
                slot_index,
                stolen_voice_id,
            }
        } else {
            AllocationResult::Unavailable
        }
    }

    /// Release a voice by note number.
    ///
    /// RT-safe: no heap allocations.
    pub fn release(&mut self, note: u8, channel: u8) {
        if let Some(slot_index) = self.note_to_slot[note as usize] {
            let slot = &mut self.slots[slot_index];

            if slot.channel == channel && slot.state == VoiceState::Active {
                // Check if sustain pedal is holding
                if self.sustain_pedal[channel as usize] {
                    slot.sustained = true;
                } else if slot.sostenuto_held {
                    // Sostenuto holds notes that were down when pedal was pressed
                    // Don't release yet
                } else {
                    slot.state = VoiceState::Releasing;
                    self.note_to_slot[note as usize] = None;
                }
            }
        }

        // Update legato tracking
        if self.config.mode == VoiceMode::Legato && self.legato_last_note == Some(note) {
            self.legato_last_note = None;
        }
    }

    /// Mark a voice as finished (call when envelope reaches zero).
    ///
    /// This frees the slot for reuse.
    pub fn voice_finished(&mut self, voice_id: VoiceId) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.voice_id == voice_id {
                // Clear note mapping if still pointing to this slot
                if self.note_to_slot[slot.note as usize] == Some(i) {
                    self.note_to_slot[slot.note as usize] = None;
                }
                slot.state = VoiceState::Idle;
                break;
            }
        }
    }

    /// Handle sustain pedal (CC 64).
    pub fn sustain_pedal(&mut self, channel: u8, on: bool) {
        if channel >= 16 {
            return;
        }

        self.sustain_pedal[channel as usize] = on;

        if !on {
            // Release all sustained notes on this channel
            for slot in &mut self.slots {
                if slot.channel == channel && slot.sustained {
                    slot.sustained = false;
                    if slot.state == VoiceState::Active && !slot.sostenuto_held {
                        slot.state = VoiceState::Releasing;
                    }
                }
            }

            // Clear note mappings for released notes
            for slot in &self.slots {
                if slot.channel == channel && slot.state == VoiceState::Releasing {
                    self.note_to_slot[slot.note as usize] = None;
                }
            }
        }
    }

    /// Handle sostenuto pedal (CC 66).
    pub fn sostenuto_pedal(&mut self, channel: u8, on: bool) {
        if channel >= 16 {
            return;
        }

        self.sostenuto_pedal[channel as usize] = on;

        if on {
            // Mark currently active notes as sostenuto held
            for slot in &mut self.slots {
                if slot.channel == channel && slot.state == VoiceState::Active {
                    slot.sostenuto_held = true;
                }
            }
        } else {
            // Release sostenuto held notes (if not also sustained)
            for (i, slot) in self.slots.iter_mut().enumerate() {
                if slot.channel == channel && slot.sostenuto_held {
                    slot.sostenuto_held = false;
                    if slot.state == VoiceState::Active && !slot.sustained {
                        // Check if key is still held
                        if self.note_to_slot[slot.note as usize] != Some(i) {
                            slot.state = VoiceState::Releasing;
                        }
                    }
                }
            }
        }
    }

    /// Update envelope level for a voice (for quietest stealing).
    pub fn update_envelope_level(&mut self, voice_id: VoiceId, level: f32) {
        for slot in &mut self.slots {
            if slot.voice_id == voice_id {
                slot.envelope_level = level;
                break;
            }
        }
    }

    /// Advance time counter (call each audio buffer).
    pub fn advance_time(&mut self, samples: u64) {
        self.current_time = self.current_time.wrapping_add(samples);
    }

    /// Get the number of active voices.
    pub fn active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state != VoiceState::Idle)
            .count()
    }

    /// Get maximum voice count.
    pub fn max_voices(&self) -> usize {
        self.config.max_voices
    }

    /// Iterate over all voice slots.
    pub fn slots(&self) -> &[VoiceSlot] {
        &self.slots
    }

    /// Get a voice slot by index.
    pub fn get_slot(&self, index: usize) -> Option<&VoiceSlot> {
        self.slots.get(index)
    }

    /// Get a mutable voice slot by index.
    pub fn get_slot_mut(&mut self, index: usize) -> Option<&mut VoiceSlot> {
        self.slots.get_mut(index)
    }

    /// Reset all voices to idle.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            *slot = VoiceSlot::default();
        }
        self.note_to_slot = [None; 128];
        self.sustain_pedal = [false; 16];
        self.sostenuto_pedal = [false; 16];
        self.legato_last_note = None;
    }

    /// All notes off for a channel.
    pub fn all_notes_off(&mut self, channel: u8) {
        for slot in self.slots.iter_mut() {
            if slot.channel == channel && slot.state == VoiceState::Active {
                slot.state = VoiceState::Releasing;
                self.note_to_slot[slot.note as usize] = None;
            }
            // Also clear sustained/sostenuto
            if slot.channel == channel {
                slot.sustained = false;
                slot.sostenuto_held = false;
            }
        }
    }

    /// All sound off for a channel (immediate silence).
    pub fn all_sound_off(&mut self, channel: u8) {
        for slot in &mut self.slots {
            if slot.channel == channel {
                slot.state = VoiceState::Idle;
                slot.sustained = false;
                slot.sostenuto_held = false;
            }
        }
        // Clear note mappings
        for i in 0..128 {
            if let Some(slot_idx) = self.note_to_slot[i] {
                if self.slots[slot_idx].channel == channel {
                    self.note_to_slot[i] = None;
                }
            }
        }
    }

    fn find_idle_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| s.state == VoiceState::Idle)
    }

    fn find_slot_to_steal(&self) -> Option<usize> {
        // First, try to steal a releasing voice
        let releasing = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.state == VoiceState::Releasing || s.state == VoiceState::Stolen);

        if let Some((i, _)) = releasing.min_by(|(_, a), (_, b)| {
            a.envelope_level
                .partial_cmp(&b.envelope_level)
                .unwrap_or(core::cmp::Ordering::Equal)
        }) {
            return Some(i);
        }

        // Otherwise use the configured strategy on active voices
        let active = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.state == VoiceState::Active);

        match self.config.strategy {
            AllocationStrategy::Oldest => active.min_by_key(|(_, s)| s.start_time).map(|(i, _)| i),
            AllocationStrategy::Quietest => active
                .min_by(|(_, a), (_, b)| {
                    a.envelope_level
                        .partial_cmp(&b.envelope_level)
                        .unwrap_or(core::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i),
            AllocationStrategy::HighestNote => active.max_by_key(|(_, s)| s.note).map(|(i, _)| i),
            AllocationStrategy::LowestNote => active.min_by_key(|(_, s)| s.note).map(|(i, _)| i),
            AllocationStrategy::Newest => active.max_by_key(|(_, s)| s.start_time).map(|(i, _)| i),
            AllocationStrategy::NoSteal => None,
        }
    }

    fn activate_slot(
        &mut self,
        slot_index: usize,
        note: u8,
        channel: u8,
        velocity: f32,
    ) -> AllocationResult {
        let voice_id = self.next_voice_id;
        self.next_voice_id += 1;

        self.slots[slot_index] = VoiceSlot {
            voice_id,
            note,
            channel,
            velocity,
            envelope_level: 0.0,
            start_time: self.current_time,
            state: VoiceState::Active,
            sustained: false,
            sostenuto_held: self.sostenuto_pedal[channel as usize],
        };

        self.note_to_slot[note as usize] = Some(slot_index);

        AllocationResult::Allocated {
            voice_id,
            slot_index,
        }
    }

    fn allocate_mono_legato(&mut self, note: u8, channel: u8, velocity: f32) -> AllocationResult {
        // Find existing active voice on this channel
        let active_slot = self
            .slots
            .iter()
            .position(|s| s.state == VoiceState::Active && s.channel == channel);

        match (self.config.mode, active_slot, self.legato_last_note) {
            (VoiceMode::Legato, Some(slot_index), Some(_)) => {
                // Legato: update note without retriggering envelope
                let voice_id = self.slots[slot_index].voice_id;
                let old_note = self.slots[slot_index].note;

                // Clear old note mapping
                self.note_to_slot[old_note as usize] = None;

                // Update slot
                self.slots[slot_index].note = note;
                self.slots[slot_index].velocity = velocity;

                // Set new note mapping
                self.note_to_slot[note as usize] = Some(slot_index);
                self.legato_last_note = Some(note);

                AllocationResult::LegatoRetrigger {
                    voice_id,
                    slot_index,
                }
            }
            _ => {
                // Mono mode or first legato note: allocate to slot 0
                // Release any existing voice first
                if let Some(slot_index) = active_slot {
                    let old_note = self.slots[slot_index].note;
                    self.note_to_slot[old_note as usize] = None;
                    self.slots[slot_index].state = VoiceState::Releasing;
                }

                self.legato_last_note = Some(note);
                self.activate_slot(0, note, channel, velocity)
            }
        }
    }
}

impl Clone for VoiceAllocator {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            slots: self.slots.clone(),
            next_voice_id: self.next_voice_id,
            current_time: self.current_time,
            note_to_slot: self.note_to_slot,
            sustain_pedal: self.sustain_pedal,
            sostenuto_pedal: self.sostenuto_pedal,
            legato_last_note: self.legato_last_note,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let config = VoiceAllocatorConfig {
            max_voices: 4,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Allocate first note
        let result = alloc.allocate(60, 0, 0.8);
        assert!(matches!(
            result,
            AllocationResult::Allocated { slot_index: 0, .. }
        ));
        assert_eq!(alloc.active_count(), 1);

        // Allocate second note
        let result = alloc.allocate(64, 0, 0.7);
        assert!(matches!(
            result,
            AllocationResult::Allocated { slot_index: 1, .. }
        ));
        assert_eq!(alloc.active_count(), 2);
    }

    #[test]
    fn test_voice_stealing_oldest() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::Oldest,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Fill all voices
        alloc.allocate(60, 0, 0.8);
        alloc.advance_time(100);
        alloc.allocate(64, 0, 0.7);
        alloc.advance_time(100);

        // Third note should steal the oldest (note 60)
        let result = alloc.allocate(67, 0, 0.9);
        assert!(matches!(
            result,
            AllocationResult::Stolen { slot_index: 0, .. }
        ));
    }

    #[test]
    fn test_release() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        alloc.allocate(60, 0, 0.8);
        assert_eq!(alloc.active_count(), 1);

        alloc.release(60, 0);
        // Voice should be releasing, not idle
        assert!(alloc.slots[0].state == VoiceState::Releasing);

        // After voice_finished, should be idle
        let voice_id = alloc.slots[0].voice_id;
        alloc.voice_finished(voice_id);
        assert!(alloc.slots[0].state == VoiceState::Idle);
    }

    #[test]
    fn test_sustain_pedal() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        alloc.allocate(60, 0, 0.8);
        alloc.sustain_pedal(0, true);
        alloc.release(60, 0);

        // Should be sustained, not releasing
        assert!(alloc.slots[0].sustained);
        assert_eq!(alloc.slots[0].state, VoiceState::Active);

        // Pedal off should release
        alloc.sustain_pedal(0, false);
        assert!(!alloc.slots[0].sustained);
        assert_eq!(alloc.slots[0].state, VoiceState::Releasing);
    }

    #[test]
    fn test_mono_mode() {
        let config = VoiceAllocatorConfig {
            max_voices: 4,
            mode: VoiceMode::Mono,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        let result1 = alloc.allocate(60, 0, 0.8);
        assert!(matches!(
            result1,
            AllocationResult::Allocated { slot_index: 0, .. }
        ));
        assert_eq!(alloc.active_count(), 1);
        assert_eq!(alloc.slots[0].note, 60);

        // Second note should retrigger in slot 0 (mono = always slot 0)
        let result2 = alloc.allocate(64, 0, 0.7);
        assert!(matches!(
            result2,
            AllocationResult::Allocated { slot_index: 0, .. }
        ));
        assert_eq!(alloc.active_count(), 1);
        assert_eq!(alloc.slots[0].note, 64);

        // Note mapping should be updated
        assert!(alloc.note_to_slot[60].is_none());
        assert_eq!(alloc.note_to_slot[64], Some(0));
    }

    #[test]
    fn test_legato_mode() {
        let config = VoiceAllocatorConfig {
            max_voices: 4,
            mode: VoiceMode::Legato,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // First note triggers normally
        let result1 = alloc.allocate(60, 0, 0.8);
        assert!(matches!(result1, AllocationResult::Allocated { .. }));

        // Second note should be legato (no retrigger)
        let result2 = alloc.allocate(64, 0, 0.7);
        assert!(matches!(result2, AllocationResult::LegatoRetrigger { .. }));

        // Should still have only 1 active voice
        assert_eq!(alloc.active_count(), 1);
        assert_eq!(alloc.slots[0].note, 64);
    }

    #[test]
    fn test_no_steal() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::NoSteal,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        alloc.allocate(60, 0, 0.8);
        alloc.allocate(64, 0, 0.7);

        // Third note should fail
        let result = alloc.allocate(67, 0, 0.9);
        assert!(matches!(result, AllocationResult::Unavailable));
    }
}
