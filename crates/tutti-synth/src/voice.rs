//! Polyphonic voice allocator with stealing strategies, mono/legato modes,
//! and sustain/sostenuto pedal handling. RT-safe after construction.

pub type VoiceId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AllocationStrategy {
    #[default]
    Oldest,
    Quietest,
    HighestNote,
    LowestNote,
    Newest,
    NoSteal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceMode {
    #[default]
    Poly,
    Mono,
    Legato,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum VoiceState {
    #[default]
    Idle,
    Active,
    Releasing,
    Stolen,
}

#[derive(Debug, Clone)]
pub(crate) struct VoiceSlot {
    pub voice_id: VoiceId,
    pub note: u8,
    pub channel: u8,
    pub velocity: f32,
    pub envelope_level: f32,
    pub start_time: u64,
    pub state: VoiceState,
    pub key_held: bool,
    pub sustained: bool,
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
            key_held: false,
            sustained: false,
            sostenuto_held: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct VoiceAllocatorConfig {
    pub max_voices: usize,
    pub strategy: AllocationStrategy,
    pub mode: VoiceMode,
}

impl Default for VoiceAllocatorConfig {
    fn default() -> Self {
        Self {
            max_voices: 16,
            strategy: AllocationStrategy::Oldest,
            mode: VoiceMode::Poly,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AllocationResult {
    Allocated { slot_index: usize },
    Stolen { slot_index: usize },
    LegatoRetrigger { slot_index: usize },
    Unavailable,
}

pub struct VoiceAllocator {
    config: VoiceAllocatorConfig,
    slots: Vec<VoiceSlot>,
    next_voice_id: VoiceId,
    current_time: u64,
    note_to_slot: [Option<usize>; 128],
    sustain_pedal: [bool; 16],
    sostenuto_pedal: [bool; 16],
    legato_last_note: Option<u8>,
}

impl VoiceAllocator {
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

    pub fn allocate(&mut self, note: u8, channel: u8, velocity: f32) -> AllocationResult {
        if self.config.mode != VoiceMode::Poly {
            return self.allocate_mono_legato(note, channel, velocity);
        }

        if let Some(existing_slot) = self.note_to_slot[note as usize] {
            self.slots[existing_slot].state = VoiceState::Releasing;
            self.note_to_slot[note as usize] = None;
        }

        if let Some(slot_index) = self.find_idle_slot() {
            return self.activate_slot(slot_index, note, channel, velocity);
        }

        if self.config.strategy == AllocationStrategy::NoSteal {
            return AllocationResult::Unavailable;
        }

        if let Some(slot_index) = self.find_slot_to_steal() {
            let old_note = self.slots[slot_index].note;
            self.note_to_slot[old_note as usize] = None;
            self.slots[slot_index].state = VoiceState::Stolen;

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
                key_held: true,
                sustained: false,
                sostenuto_held: false,
            };

            self.note_to_slot[note as usize] = Some(slot_index);

            AllocationResult::Stolen { slot_index }
        } else {
            AllocationResult::Unavailable
        }
    }

    pub fn release(&mut self, note: u8, channel: u8) {
        if let Some(slot_index) = self.note_to_slot[note as usize] {
            let slot = &mut self.slots[slot_index];

            if slot.channel == channel && slot.state == VoiceState::Active {
                slot.key_held = false;
                if self.sustain_pedal[channel as usize] {
                    slot.sustained = true;
                } else if slot.sostenuto_held {
                    // Sostenuto holds — don't release yet
                } else {
                    slot.state = VoiceState::Releasing;
                    self.note_to_slot[note as usize] = None;
                }
            }
        }

        if self.config.mode == VoiceMode::Legato && self.legato_last_note == Some(note) {
            self.legato_last_note = None;
        }
    }

    /// Call when envelope reaches zero to free the slot for reuse.
    pub fn voice_finished(&mut self, voice_id: VoiceId) {
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if slot.voice_id == voice_id {
                if self.note_to_slot[slot.note as usize] == Some(i) {
                    self.note_to_slot[slot.note as usize] = None;
                }
                slot.state = VoiceState::Idle;
                break;
            }
        }
    }

    /// CC 64.
    pub fn sustain_pedal(&mut self, channel: u8, on: bool) {
        if channel >= 16 {
            return;
        }

        self.sustain_pedal[channel as usize] = on;

        if !on {
            // Release sustained notes (unless key is still held or sostenuto holds)
            for slot in &mut self.slots {
                if slot.channel == channel && slot.sustained {
                    slot.sustained = false;
                    if slot.state == VoiceState::Active
                        && !slot.sostenuto_held
                        && !slot.key_held
                    {
                        slot.state = VoiceState::Releasing;
                        self.note_to_slot[slot.note as usize] = None;
                    }
                }
            }
        }
    }

    /// CC 66.
    pub fn sostenuto_pedal(&mut self, channel: u8, on: bool) {
        if channel >= 16 {
            return;
        }

        self.sostenuto_pedal[channel as usize] = on;

        if on {
            for slot in &mut self.slots {
                if slot.channel == channel && slot.state == VoiceState::Active {
                    slot.sostenuto_held = true;
                }
            }
        } else {
            // Release sostenuto held notes (if key is no longer held and not sustained)
            for slot in self.slots.iter_mut() {
                if slot.channel == channel && slot.sostenuto_held {
                    slot.sostenuto_held = false;
                    if slot.state == VoiceState::Active && !slot.sustained && !slot.key_held {
                        slot.state = VoiceState::Releasing;
                        self.note_to_slot[slot.note as usize] = None;
                    }
                }
            }
        }
    }

    /// Used by quietest-voice stealing strategy.
    pub fn update_envelope_level(&mut self, slot_index: usize, level: f32) {
        if slot_index < self.slots.len() {
            self.slots[slot_index].envelope_level = level;
        }
    }

    /// Call once per audio buffer.
    pub fn advance_time(&mut self, samples: u64) {
        self.current_time = self.current_time.wrapping_add(samples);
    }

    #[cfg(test)]
    pub fn active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|s| s.state != VoiceState::Idle)
            .count()
    }

    pub(crate) fn slots(&self) -> &[VoiceSlot] {
        &self.slots
    }

    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            *slot = VoiceSlot::default();
        }
        self.note_to_slot = [None; 128];
        self.sustain_pedal = [false; 16];
        self.sostenuto_pedal = [false; 16];
        self.legato_last_note = None;
    }

    pub fn all_notes_off(&mut self, channel: u8) {
        for slot in self.slots.iter_mut() {
            if slot.channel == channel && slot.state == VoiceState::Active {
                slot.state = VoiceState::Releasing;
                self.note_to_slot[slot.note as usize] = None;
            }
            if slot.channel == channel {
                slot.sustained = false;
                slot.sostenuto_held = false;
            }
        }
    }

    /// Immediate silence, unlike `all_notes_off` which allows release tails.
    pub fn all_sound_off(&mut self, channel: u8) {
        for slot in &mut self.slots {
            if slot.channel == channel {
                slot.state = VoiceState::Idle;
                slot.sustained = false;
                slot.sostenuto_held = false;
            }
        }
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
            key_held: true,
            sustained: false,
            sostenuto_held: false,
        };

        self.note_to_slot[note as usize] = Some(slot_index);

        AllocationResult::Allocated { slot_index }
    }

    fn allocate_mono_legato(&mut self, note: u8, channel: u8, velocity: f32) -> AllocationResult {
        let active_slot = self
            .slots
            .iter()
            .position(|s| s.state == VoiceState::Active && s.channel == channel);

        match (self.config.mode, active_slot, self.legato_last_note) {
            (VoiceMode::Legato, Some(slot_index), Some(_)) => {
                // Legato: update note without retriggering envelope
                let old_note = self.slots[slot_index].note;
                self.note_to_slot[old_note as usize] = None;
                self.slots[slot_index].note = note;
                self.slots[slot_index].velocity = velocity;
                self.note_to_slot[note as usize] = Some(slot_index);
                self.legato_last_note = Some(note);

                AllocationResult::LegatoRetrigger { slot_index }
            }
            _ => {
                // Mono or first legato note: always use slot 0
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
            AllocationResult::Allocated { slot_index: 0 }
        ));
        assert_eq!(alloc.active_count(), 1);

        // Allocate second note
        let result = alloc.allocate(64, 0, 0.7);
        assert!(matches!(
            result,
            AllocationResult::Allocated { slot_index: 1 }
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
            AllocationResult::Stolen { slot_index: 0 }
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
            AllocationResult::Allocated { slot_index: 0 }
        ));
        assert_eq!(alloc.active_count(), 1);
        assert_eq!(alloc.slots[0].note, 60);

        // Second note should retrigger in slot 0 (mono = always slot 0)
        let result2 = alloc.allocate(64, 0, 0.7);
        assert!(matches!(
            result2,
            AllocationResult::Allocated { slot_index: 0 }
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

    #[test]
    fn test_voice_stealing_quietest() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::Quietest,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Allocate two voices
        let result1 = alloc.allocate(60, 0, 0.8);
        let slot_1 = match result1 {
            AllocationResult::Allocated { slot_index } => slot_index,
            _ => panic!("Expected allocation"),
        };

        let result2 = alloc.allocate(64, 0, 0.7);
        let slot_2 = match result2 {
            AllocationResult::Allocated { slot_index } => slot_index,
            _ => panic!("Expected allocation"),
        };

        // Set envelope levels - slot 0 is quieter
        alloc.update_envelope_level(slot_1, 0.2);
        alloc.update_envelope_level(slot_2, 0.8);

        // Third note should steal the quietest (slot 0, note 60)
        let result = alloc.allocate(67, 0, 0.9);
        match result {
            AllocationResult::Stolen { slot_index } => {
                assert_eq!(slot_index, 0);
            }
            _ => panic!("Expected stealing, got {:?}", result),
        }
    }

    #[test]
    fn test_voice_stealing_highest_note() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::HighestNote,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Allocate C4 (60) and G4 (67)
        alloc.allocate(60, 0, 0.8); // slot 0
        alloc.allocate(67, 0, 0.8); // slot 1 - higher note

        // Third note should steal the highest (G4 = 67)
        let result = alloc.allocate(72, 0, 0.9);
        match result {
            AllocationResult::Stolen { slot_index } => {
                assert_eq!(slot_index, 1, "Should steal slot with highest note (67)");
            }
            _ => panic!("Expected stealing"),
        }
    }

    #[test]
    fn test_voice_stealing_lowest_note() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::LowestNote,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Allocate C4 (60) and G4 (67)
        alloc.allocate(60, 0, 0.8); // slot 0 - lower note
        alloc.allocate(67, 0, 0.8); // slot 1

        // Third note should steal the lowest (C4 = 60)
        let result = alloc.allocate(72, 0, 0.9);
        match result {
            AllocationResult::Stolen { slot_index } => {
                assert_eq!(slot_index, 0, "Should steal slot with lowest note (60)");
            }
            _ => panic!("Expected stealing"),
        }
    }

    #[test]
    fn test_voice_stealing_newest() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::Newest,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        alloc.allocate(60, 0, 0.8); // slot 0 - older
        alloc.advance_time(100);
        alloc.allocate(64, 0, 0.7); // slot 1 - newer
        alloc.advance_time(100);

        // Third note should steal the newest (note 64 in slot 1)
        let result = alloc.allocate(67, 0, 0.9);
        match result {
            AllocationResult::Stolen { slot_index } => {
                assert_eq!(slot_index, 1, "Should steal newest voice");
            }
            _ => panic!("Expected stealing"),
        }
    }

    #[test]
    fn test_sostenuto_pedal() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play note, then press sostenuto
        alloc.allocate(60, 0, 0.8);
        alloc.sostenuto_pedal(0, true);

        // Release key - should be held by sostenuto
        alloc.release(60, 0);
        assert_eq!(alloc.slots[0].state, VoiceState::Active);
        assert!(alloc.slots[0].sostenuto_held);

        // New note played AFTER sostenuto down should NOT be held
        alloc.allocate(64, 0, 0.7);
        assert!(!alloc.slots[1].sostenuto_held, "New notes should not be sostenuto-held");
        alloc.release(64, 0);
        assert_eq!(alloc.slots[1].state, VoiceState::Releasing, "New note should release normally");

        // Original note still held
        assert_eq!(alloc.slots[0].state, VoiceState::Active);

        // Sostenuto off should release the held note (key was released)
        alloc.sostenuto_pedal(0, false);
        assert!(!alloc.slots[0].sostenuto_held);
        assert_eq!(alloc.slots[0].state, VoiceState::Releasing);
    }

    #[test]
    fn test_sostenuto_key_still_held() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play note, then press sostenuto
        alloc.allocate(60, 0, 0.8);
        alloc.sostenuto_pedal(0, true);

        // DON'T release key — sostenuto off should NOT release
        alloc.sostenuto_pedal(0, false);
        assert_eq!(alloc.slots[0].state, VoiceState::Active);
        assert!(!alloc.slots[0].sostenuto_held);
        assert!(alloc.slots[0].key_held);
    }

    #[test]
    fn test_all_notes_off() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play notes on channel 0 and 1
        alloc.allocate(60, 0, 0.8);
        alloc.allocate(64, 0, 0.7);
        alloc.allocate(67, 1, 0.9);

        assert_eq!(alloc.active_count(), 3);

        // All notes off on channel 0 only
        alloc.all_notes_off(0);

        // Channel 0 notes should be releasing
        assert_eq!(alloc.slots[0].state, VoiceState::Releasing);
        assert_eq!(alloc.slots[1].state, VoiceState::Releasing);
        // Channel 1 note still active
        assert_eq!(alloc.slots[2].state, VoiceState::Active);
    }

    #[test]
    fn test_all_sound_off() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play notes
        alloc.allocate(60, 0, 0.8);
        alloc.allocate(64, 0, 0.7);

        // All sound off - immediate silence
        alloc.all_sound_off(0);

        // Should be immediately idle (no release phase)
        assert_eq!(alloc.slots[0].state, VoiceState::Idle);
        assert_eq!(alloc.slots[1].state, VoiceState::Idle);
        assert_eq!(alloc.active_count(), 0);
    }

    #[test]
    fn test_reset() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play notes with pedals
        alloc.allocate(60, 0, 0.8);
        alloc.allocate(64, 0, 0.7);
        alloc.sustain_pedal(0, true);
        alloc.sostenuto_pedal(0, true);

        assert_eq!(alloc.active_count(), 2);

        // Reset everything
        alloc.reset();

        assert_eq!(alloc.active_count(), 0);
        // Pedals should be cleared
        alloc.allocate(60, 0, 0.8);
        alloc.release(60, 0);
        // Without sustain, should go to releasing
        assert_eq!(alloc.slots[0].state, VoiceState::Releasing);
    }

    #[test]
    fn test_retrigger_same_note() {
        let config = VoiceAllocatorConfig::default();
        let mut alloc = VoiceAllocator::new(config);

        // Play same note twice
        alloc.allocate(60, 0, 0.8);
        assert_eq!(alloc.slots[0].state, VoiceState::Active);

        // Same note again should release old and allocate new
        let result = alloc.allocate(60, 0, 0.9);
        match result {
            AllocationResult::Allocated { slot_index } => {
                // Old voice should be releasing, new one allocated
                assert!(slot_index == 0 || slot_index == 1);
            }
            _ => panic!("Expected new allocation"),
        }
    }

    #[test]
    fn test_prefer_stealing_releasing_voices() {
        let config = VoiceAllocatorConfig {
            max_voices: 2,
            strategy: AllocationStrategy::Oldest,
            ..Default::default()
        };
        let mut alloc = VoiceAllocator::new(config);

        // Allocate and release one voice
        alloc.allocate(60, 0, 0.8);
        alloc.advance_time(100);
        alloc.allocate(64, 0, 0.7);
        alloc.advance_time(100);
        alloc.release(60, 0); // Now slot 0 is releasing

        // New note should prefer the releasing voice over active
        let result = alloc.allocate(67, 0, 0.9);
        match result {
            AllocationResult::Stolen { slot_index } => {
                assert_eq!(slot_index, 0, "Should prefer stealing releasing voice");
            }
            _ => panic!("Expected stealing"),
        }
    }
}
