//! Transport manager with FSM-based state management.

use std::sync::Arc;
use std::cell::UnsafeCell;
use arc_swap::ArcSwap;
use crossbeam_channel::{Sender, Receiver, unbounded};

use super::tempo_map::{TempoMap, TempoMapSnapshot, TimeSignature, BBT};
use super::fsm::{TransportFSM, TransportEvent};
use super::position::{MusicalPosition, LoopRange};
use crate::{AtomicDouble, AtomicFlag, AtomicFloat, AtomicU8};
use std::sync::atomic::Ordering;

// Re-export MotionState from FSM
pub use super::fsm::MotionState;

impl MotionState {
    fn to_u8(self) -> u8 {
        match self {
            MotionState::Stopped => 0,
            MotionState::Rolling => 1,
            MotionState::FastForward => 2,
            MotionState::Rewind => 3,
            MotionState::DeclickToStop => 4,
            MotionState::DeclickToLocate => 5,
        }
    }

    fn from_u8(val: u8) -> Self {
        match val {
            1 => MotionState::Rolling,
            2 => MotionState::FastForward,
            3 => MotionState::Rewind,
            4 => MotionState::DeclickToStop,
            5 => MotionState::DeclickToLocate,
            _ => MotionState::Stopped,
        }
    }
}

/// Transport manager - lock-free command queue for FSM updates.
pub struct TransportManager {
    // Lock-free command queue (UI thread sends, processor thread receives)
    command_tx: Sender<TransportEvent>,
    command_rx: Receiver<TransportEvent>,

    // FSM state (owned by audio thread, accessed via UnsafeCell)
    fsm: UnsafeCell<TransportFSM>,

    // Atomic mirrors for RT reads (updated by FSM processor)
    tempo: Arc<AtomicFloat>,
    paused: Arc<AtomicFlag>,
    current_beat: Arc<AtomicDouble>,
    loop_enabled: Arc<AtomicFlag>,
    loop_start_beat: Arc<AtomicDouble>,
    loop_end_beat: Arc<AtomicDouble>,
    motion_state: Arc<AtomicU8>,

    // Tempo map
    tempo_map: Arc<ArcSwap<TempoMap>>,
    tempo_map_shared: Arc<ArcSwap<TempoMapSnapshot>>,

    sample_rate: f64,
}

// SAFETY: TransportManager is safe to send/sync because:
// - command_tx/rx are lock-free channels (Send + Sync)
// - fsm is only accessed from audio thread via process_commands()
// - All other fields are already Send + Sync (atomics, Arc)
unsafe impl Send for TransportManager {}
unsafe impl Sync for TransportManager {}

impl TransportManager {
    pub(crate) fn new(sample_rate: f64) -> Self {
        let tempo_map = TempoMap::new(120.0, sample_rate);
        let tempo_map_shared = Arc::new(ArcSwap::new(tempo_map.snapshot()));

        let (command_tx, command_rx) = unbounded();
        let fsm = UnsafeCell::new(TransportFSM::new());

        Self {
            command_tx,
            command_rx,
            fsm,
            tempo: Arc::new(AtomicFloat::new(120.0)),
            paused: Arc::new(AtomicFlag::new(true)),
            current_beat: Arc::new(AtomicDouble::new(0.0)),
            loop_enabled: Arc::new(AtomicFlag::new(false)),
            loop_start_beat: Arc::new(AtomicDouble::new(0.0)),
            loop_end_beat: Arc::new(AtomicDouble::new(16.0)),
            motion_state: Arc::new(AtomicU8::new(MotionState::Stopped.to_u8())),
            tempo_map: Arc::new(ArcSwap::new(Arc::new(tempo_map))),
            tempo_map_shared,
            sample_rate,
        }
    }

    /// Process pending transport commands (call from audio thread).
    ///
    /// SAFETY: This must only be called from the audio thread. The FSM is accessed
    /// via UnsafeCell to allow mutation through &self (since TransportManager is in Arc).
    pub fn process_commands(&self) {
        while let Ok(event) = self.command_rx.try_recv() {
            let fsm = unsafe { &mut *self.fsm.get() };
            let result = fsm.transition(event);
            self.apply_fsm_result(result);
        }
    }

    /// Send transport command (lock-free, safe from any thread).
    fn send_command(&self, event: TransportEvent) {
        let _ = self.command_tx.send(event);
    }

    pub fn tempo(&self) -> &Arc<AtomicFloat> {
        &self.tempo
    }

    pub fn current_beat(&self) -> &Arc<AtomicDouble> {
        &self.current_beat
    }

    pub fn tempo_map_shared(&self) -> &Arc<ArcSwap<TempoMapSnapshot>> {
        &self.tempo_map_shared
    }

    pub fn get_tempo(&self) -> f32 {
        self.tempo.get()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.get()
    }

    pub fn get_current_beat(&self) -> f64 {
        self.current_beat.get()
    }

    pub fn is_loop_enabled(&self) -> bool {
        self.loop_enabled.get()
    }

    pub fn get_loop_range(&self) -> Option<(f64, f64)> {
        if self.loop_enabled.get() {
            Some((self.loop_start_beat.get(), self.loop_end_beat.get()))
        } else {
            None
        }
    }

    pub fn set_tempo(&self, bpm: f32) {
        self.tempo.set(bpm);
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.set(paused);
    }

    pub fn set_current_beat(&self, beat: f64) {
        self.current_beat.set(beat);
    }

    /// Advance position (called from audio callback - RT-safe).
    /// Returns true if loop boundary was hit.
    pub fn advance_position_rt(&self, beat_increment: f64) -> bool {
        let current = self.current_beat.get();
        let new_position = current + beat_increment;

        // Check for loop wrap
        if self.loop_enabled.get() {
            let start = self.loop_start_beat.get();
            let end = self.loop_end_beat.get();
            if new_position >= end {
                // Wrap to loop start
                let wrapped = start + (new_position - end);
                self.current_beat.set(wrapped);
                return true;
            }
        }

        self.current_beat.set(new_position);
        false
    }

    // FSM-based transport commands (lock-free via command queue)

    /// Start playback.
    pub fn play(&self) {
        self.send_command(TransportEvent::Play);
    }

    /// Stop playback with declick.
    pub fn stop(&self) {
        self.send_command(TransportEvent::StopWithDeclick);
    }

    /// Stop playback immediately (no declick).
    pub fn stop_immediate(&self) {
        self.send_command(TransportEvent::Stop);
    }

    /// Locate to a position in beats.
    pub fn locate(&self, beats: f64) {
        let position = MusicalPosition::from_beats(beats);
        self.send_command(TransportEvent::Locate(position));
    }

    /// Locate to a position and start playing.
    pub fn locate_and_play(&self, beats: f64) {
        let position = MusicalPosition::from_beats(beats);
        self.send_command(TransportEvent::LocateAndPlay(position));
    }

    /// Locate with declick (smooth seeking).
    pub fn locate_with_declick(&self, beats: f64) {
        let position = MusicalPosition::from_beats(beats);
        self.send_command(TransportEvent::LocateWithDeclick(position));
    }

    /// Toggle loop enabled/disabled.
    pub fn toggle_loop(&self) {
        self.send_command(TransportEvent::ToggleLoop);
    }

    /// Set loop range (start and end in beats).
    pub fn set_loop_range_fsm(&self, start: f64, end: f64) {
        let range = LoopRange::new(start, end);
        self.send_command(TransportEvent::SetLoopRange(range));
    }

    /// Clear loop range.
    pub fn clear_loop(&self) {
        self.send_command(TransportEvent::ClearLoop);
    }

    /// Start fast forward.
    pub fn fast_forward(&self) {
        self.send_command(TransportEvent::FastForward);
    }

    /// Start rewind.
    pub fn rewind(&self) {
        self.send_command(TransportEvent::Rewind);
    }

    /// End scrub/shuttle mode.
    pub fn end_scrub(&self) {
        self.send_command(TransportEvent::EndScrub);
    }

    /// Toggle reverse playback.
    pub fn reverse(&self) {
        self.send_command(TransportEvent::Reverse);
    }

    /// Get current motion state (RT-safe).
    pub fn motion_state(&self) -> MotionState {
        MotionState::from_u8(self.motion_state.load(Ordering::Acquire))
    }

    /// Apply FSM transition result to atomic state.
    fn apply_fsm_result(&self, result: super::fsm::TransitionResult) {
        use super::fsm::TransitionResult;

        match result {
            TransitionResult::MotionChanged(motion) => {
                self.motion_state.store(motion.to_u8(), Ordering::Release);
                // Update paused flag based on motion state
                let paused = matches!(motion, MotionState::Stopped | MotionState::DeclickToStop);
                self.paused.set(paused);
            }
            TransitionResult::Locating(pos) => {
                self.current_beat.set(pos.beats);
            }
            TransitionResult::LoopModeChanged(enabled) => {
                self.loop_enabled.set(enabled);
            }
            _ => {}
        }
    }

    pub fn set_loop_enabled(&self, enabled: bool) {
        self.loop_enabled.set(enabled);
    }

    pub fn set_loop_range(&self, start: f64, end: f64) {
        self.loop_start_beat.set(start);
        self.loop_end_beat.set(end);
    }

    pub fn tempo_map_snapshot(&self) -> Arc<TempoMapSnapshot> {
        self.tempo_map.load().snapshot()
    }

    fn publish_tempo_map(&self) {
        let tempo_map = self.tempo_map.load();
        self.tempo_map_shared.store(tempo_map.snapshot());
    }

    pub fn add_tempo_point(&self, beat: f64, bpm: f32) {
        // Clone current tempo map, modify, and swap atomically
        let mut new_map = (**self.tempo_map.load()).clone();
        new_map.add_tempo_point(beat, bpm);
        self.tempo_map.store(Arc::new(new_map));
        self.publish_tempo_map();
    }

    pub fn remove_tempo_point(&self, beat: f64) {
        // Clone current tempo map, modify, and swap atomically
        let mut new_map = (**self.tempo_map.load()).clone();
        new_map.remove_tempo_point(beat);
        self.tempo_map.store(Arc::new(new_map));
        self.publish_tempo_map();
    }

    pub fn clear_tempo_automation(&self) {
        // Clone current tempo map, modify, and swap atomically
        let mut new_map = (**self.tempo_map.load()).clone();
        new_map.clear_tempo_automation();
        self.tempo_map.store(Arc::new(new_map));
        self.publish_tempo_map();
    }

    pub fn set_time_signature(&self, numerator: u32, denominator: u32) {
        // Clone current tempo map, modify, and swap atomically
        let mut new_map = (**self.tempo_map.load()).clone();
        new_map.set_time_signature(numerator, denominator);
        self.tempo_map.store(Arc::new(new_map));
        self.publish_tempo_map();
    }

    pub fn time_signature(&self) -> TimeSignature {
        self.tempo_map.load().time_signature()
    }

    pub fn beats_to_bbt(&self, beats: f64) -> BBT {
        self.tempo_map.load().beats_to_bbt(beats)
    }

    pub fn bbt_to_beats(&self, bbt: BBT) -> f64 {
        self.tempo_map.load().bbt_to_beats(bbt)
    }

    pub fn beats_to_seconds(&self, beats: f64) -> f64 {
        self.tempo_map.load().beats_to_seconds(beats)
    }

    pub fn seconds_to_beats(&self, seconds: f64) -> f64 {
        self.tempo_map.load().seconds_to_beats(seconds)
    }

    pub fn beats_to_samples(&self, beats: f64) -> u64 {
        self.tempo_map.load().beats_to_samples(beats)
    }

    pub fn samples_to_beats(&self, samples: u64) -> f64 {
        self.tempo_map.load().samples_to_beats(samples)
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn beats_per_second(&self) -> f64 {
        self.tempo.get() as f64 / 60.0
    }

    pub fn samples_per_beat(&self) -> f64 {
        self.sample_rate / self.beats_per_second()
    }

    // REMOVED: Direct tempo map access - use tempo_map_snapshot() instead
    // TempoMap is now wrapped in ArcSwap for lock-free copy-on-write updates
}

impl Default for TransportManager {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_default_state() {
        let manager = TransportManager::new(48000.0);

        assert_eq!(manager.get_tempo(), 120.0);
        assert_eq!(manager.is_paused(), true);
        assert_eq!(manager.get_current_beat(), 0.0);
        assert_eq!(manager.is_loop_enabled(), false);
        assert_eq!(manager.sample_rate(), 48000.0);
    }

    #[test]
    fn test_atomic_accessors() {
        let manager = TransportManager::new(48000.0);

        assert_eq!(manager.tempo().get(), manager.get_tempo());
        assert_eq!(manager.current_beat().get(), manager.get_current_beat());
    }

    #[test]
    fn test_tempo_changes() {
        let manager = TransportManager::new(48000.0);

        manager.set_tempo(140.0);
        assert_eq!(manager.get_tempo(), 140.0);
        assert_eq!(manager.tempo().get(), 140.0);

        manager.set_tempo(80.0);
        assert_eq!(manager.get_tempo(), 80.0);
    }

    #[test]
    fn test_playback_state() {
        let manager = TransportManager::new(48000.0);

        assert_eq!(manager.is_paused(), true);

        manager.set_paused(false);
        assert_eq!(manager.is_paused(), false);

        manager.set_paused(true);
        assert_eq!(manager.is_paused(), true);
    }

    #[test]
    fn test_position_changes() {
        let manager = TransportManager::new(48000.0);

        manager.set_current_beat(4.5);
        assert_eq!(manager.get_current_beat(), 4.5);
        assert_eq!(manager.current_beat().get(), 4.5);

        manager.set_current_beat(0.0);
        assert_eq!(manager.get_current_beat(), 0.0);
    }

    #[test]
    fn test_loop_range() {
        let manager = TransportManager::new(48000.0);

        // Initially disabled
        assert_eq!(manager.is_loop_enabled(), false);
        assert_eq!(manager.get_loop_range(), None);

        manager.set_loop_range(2.0, 8.0);
        manager.set_loop_enabled(true);

        assert_eq!(manager.is_loop_enabled(), true);
        assert_eq!(manager.get_loop_range(), Some((2.0, 8.0)));

        // Disable loop
        manager.set_loop_enabled(false);
        assert_eq!(manager.get_loop_range(), None);
    }

    #[test]
    fn test_tempo_map_conversions() {
        let manager = TransportManager::new(48000.0);

        // Test basic conversions at 120 BPM
        let beats = 4.0;
        let seconds = manager.beats_to_seconds(beats);
        let beats_back = manager.seconds_to_beats(seconds);

        assert!((beats - beats_back).abs() < 0.0001);
    }

    #[test]
    fn test_beats_per_second() {
        let manager = TransportManager::new(48000.0);

        // 120 BPM = 2 beats per second
        assert!((manager.beats_per_second() - 2.0).abs() < 0.0001);

        manager.set_tempo(60.0);
        // 60 BPM = 1 beat per second
        assert!((manager.beats_per_second() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_samples_per_beat() {
        let manager = TransportManager::new(48000.0);

        // 120 BPM at 48kHz = 24000 samples per beat
        assert!((manager.samples_per_beat() - 24000.0).abs() < 0.1);

        manager.set_tempo(60.0);
        // 60 BPM at 48kHz = 48000 samples per beat
        assert!((manager.samples_per_beat() - 48000.0).abs() < 0.1);
    }

    #[test]
    fn test_tempo_automation() {
        let manager = TransportManager::new(48000.0);

        manager.add_tempo_point(8.0, 140.0);

        // Tempo changes take effect, so conversion should differ
        let t1 = manager.beats_to_seconds(4.0); // Before tempo change
        let t2 = manager.beats_to_seconds(12.0); // After tempo change
        assert!(t2 > t1);
    }

    #[test]
    fn test_time_signature() {
        let manager = TransportManager::new(48000.0);

        // Default is 4/4
        let sig = manager.time_signature();
        assert_eq!(sig.numerator, 4);
        assert_eq!(sig.denominator, 4);

        // Change to 3/4
        manager.set_time_signature(3, 4);
        let sig = manager.time_signature();
        assert_eq!(sig.numerator, 3);
        assert_eq!(sig.denominator, 4);
    }

    #[test]
    fn test_bbt_conversion() {
        let manager = TransportManager::new(48000.0);

        // 4 beats at 4/4 = bar 2, beat 1
        let bbt = manager.beats_to_bbt(4.0);
        assert_eq!(bbt.bar, 2);
        assert_eq!(bbt.beat, 1);

        // Convert back
        let beats = manager.bbt_to_beats(bbt);
        assert!((beats - 4.0).abs() < 0.0001);
    }

    #[test]
    fn test_concurrent_access() {
        use std::sync::Arc;
        use std::thread;

        let manager = Arc::new(TransportManager::new(48000.0));

        // Spawn threads that read atomics
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let m = manager.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        let _ = m.get_tempo();
                        let _ = m.is_paused();
                        let _ = m.get_current_beat();
                        let _ = m.is_loop_enabled();
                    }
                })
            })
            .collect();

        // Main thread writes
        for i in 0..100 {
            manager.set_tempo(100.0 + i as f32);
            manager.set_current_beat(i as f64);
        }

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    }
}
