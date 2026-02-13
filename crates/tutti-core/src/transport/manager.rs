//! Transport manager with FSM-based state management.

use crate::compat::{Arc, UnsafeCell};
use arc_swap::ArcSwap;
use crossbeam_channel::{unbounded, Receiver, Sender};

use super::fsm::{TransportEvent, TransportFSM};
use super::position::{LoopRange, MusicalPosition};
use super::sync::{SyncSnapshot, SyncSource, SyncState};
use super::tempo_map::{TempoMap, TempoMapSnapshot, TimeSignature, BBT};
use crate::compat::Ordering;
use crate::{AtomicDouble, AtomicFlag, AtomicFloat, AtomicU8};

pub use super::fsm::{Direction, MotionState};

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
    command_tx: Sender<TransportEvent>,
    command_rx: Receiver<TransportEvent>,
    fsm: UnsafeCell<TransportFSM>,
    tempo: Arc<AtomicFloat>,
    paused: Arc<AtomicFlag>,
    reverse: Arc<AtomicFlag>,
    recording: Arc<AtomicFlag>,
    in_preroll: Arc<AtomicFlag>,
    current_beat: Arc<AtomicDouble>,
    loop_enabled: Arc<AtomicFlag>,
    loop_start_beat: Arc<AtomicDouble>,
    loop_end_beat: Arc<AtomicDouble>,
    motion_state: Arc<AtomicU8>,
    seek_target: Arc<AtomicDouble>,
    seek_pending: Arc<AtomicFlag>,

    // Tempo map
    tempo_map: Arc<ArcSwap<TempoMap>>,
    tempo_map_shared: Arc<ArcSwap<TempoMapSnapshot>>,

    // External sync state
    sync_state: Arc<SyncState>,

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
            reverse: Arc::new(AtomicFlag::new(false)),
            recording: Arc::new(AtomicFlag::new(false)),
            in_preroll: Arc::new(AtomicFlag::new(false)),
            current_beat: Arc::new(AtomicDouble::new(0.0)),
            loop_enabled: Arc::new(AtomicFlag::new(false)),
            loop_start_beat: Arc::new(AtomicDouble::new(0.0)),
            loop_end_beat: Arc::new(AtomicDouble::new(16.0)),
            motion_state: Arc::new(AtomicU8::new(MotionState::Stopped.to_u8())),
            seek_target: Arc::new(AtomicDouble::new(0.0)),
            seek_pending: Arc::new(AtomicFlag::new(false)),
            tempo_map: Arc::new(ArcSwap::new(Arc::new(tempo_map))),
            tempo_map_shared,
            sync_state: Arc::new(SyncState::new()),
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
            if let Some(result) = fsm.transition(event) {
                self.apply_fsm_result(result);
            }
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

    /// Get the paused flag Arc for sharing with ClickState.
    pub fn paused(&self) -> &Arc<AtomicFlag> {
        &self.paused
    }

    /// Get the recording flag Arc for sharing with ClickState.
    pub fn recording(&self) -> &Arc<AtomicFlag> {
        &self.recording
    }

    /// Get the preroll flag Arc for sharing with ClickState.
    pub fn in_preroll(&self) -> &Arc<AtomicFlag> {
        &self.in_preroll
    }

    /// Get the loop enabled flag Arc for sharing with TransportClock.
    pub fn loop_enabled_flag(&self) -> &Arc<AtomicFlag> {
        &self.loop_enabled
    }

    /// Get the loop start beat Arc for sharing with TransportClock.
    pub fn loop_start_beat_atomic(&self) -> &Arc<AtomicDouble> {
        &self.loop_start_beat
    }

    /// Get the loop end beat Arc for sharing with TransportClock.
    pub fn loop_end_beat_atomic(&self) -> &Arc<AtomicDouble> {
        &self.loop_end_beat
    }

    /// Get the seek target Arc for sharing with TransportClock.
    pub fn seek_target(&self) -> &Arc<AtomicDouble> {
        &self.seek_target
    }

    /// Get the seek pending flag Arc for sharing with TransportClock.
    pub fn seek_pending(&self) -> &Arc<AtomicFlag> {
        &self.seek_pending
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

    /// Check if recording is active.
    pub fn is_recording(&self) -> bool {
        self.recording.get()
    }

    /// Check if in preroll count-in.
    pub fn is_in_preroll(&self) -> bool {
        self.in_preroll.get()
    }

    /// Check if playback is in reverse direction.
    pub fn is_reverse(&self) -> bool {
        self.reverse.get()
    }

    /// Get current playback direction.
    pub fn direction(&self) -> Direction {
        if self.reverse.get() {
            Direction::Backwards
        } else {
            Direction::Forwards
        }
    }

    pub fn get_current_beat(&self) -> f64 {
        self.current_beat.get()
    }

    pub fn is_loop_enabled(&self) -> bool {
        self.loop_enabled.get()
    }

    pub fn get_loop_range(&self) -> Option<(f64, f64)> {
        self.loop_enabled
            .get()
            .then(|| (self.loop_start_beat.get(), self.loop_end_beat.get()))
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

    /// Set recording state.
    pub fn set_recording(&self, recording: bool) {
        self.recording.set(recording);
    }

    /// Set preroll state.
    pub fn set_in_preroll(&self, in_preroll: bool) {
        self.in_preroll.set(in_preroll);
    }

    /// Advance position (called from audio callback - RT-safe).
    /// Returns true if loop boundary was hit.
    pub fn advance_position_rt(&self, beat_increment: f64) -> bool {
        let current = self.current_beat.get();
        let reverse = self.reverse.get();

        let new_position = if reverse {
            current - beat_increment
        } else {
            current + beat_increment
        };

        // Check for loop wrap
        if self.loop_enabled.get() {
            let start = self.loop_start_beat.get();
            let end = self.loop_end_beat.get();

            if reverse {
                // Reverse: wrap at loop start
                if new_position < start {
                    let wrapped = end - (start - new_position);
                    self.current_beat.set(wrapped);
                    return true;
                }
            } else {
                // Forward: wrap at loop end
                if new_position >= end {
                    let wrapped = start + (new_position - end);
                    self.current_beat.set(wrapped);
                    return true;
                }
            }
        } else if reverse && new_position < 0.0 {
            // Clamp at 0 when not looping in reverse
            self.current_beat.set(0.0);
            return false;
        }

        self.current_beat.set(new_position);
        false
    }

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
        // Flip atomic immediately so readers see the change right away,
        // then sync the FSM on the next audio callback.
        let current = self.loop_enabled.get();
        self.loop_enabled.set(!current);
        self.send_command(TransportEvent::SetLoopEnabled(!current));
    }

    /// Set loop range (start and end in beats).
    pub fn set_loop_range_fsm(&self, start: f64, end: f64) {
        // Set atomics immediately so advance_position_rt sees them right away
        self.loop_start_beat.set(start);
        self.loop_end_beat.set(end);
        self.loop_enabled.set(true);
        let range = LoopRange::new(start, end);
        self.send_command(TransportEvent::SetLoopRange(range));
    }

    /// Clear loop range.
    pub fn clear_loop(&self) {
        self.loop_enabled.set(false);
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
            TransitionResult::DeclickStarted => {
                // When declick starts (for stop or locate), set paused to stop audio immediately
                self.motion_state
                    .store(MotionState::DeclickToStop.to_u8(), Ordering::Release);
                self.paused.set(true);
            }
            TransitionResult::Locating(pos) => {
                self.current_beat.set(pos.beats);
                self.seek_target.set(pos.beats);
                self.seek_pending.set(true);
            }
            TransitionResult::LoopModeChanged(enabled) => {
                self.loop_enabled.set(enabled);
            }
            TransitionResult::DirectionChanged(direction) => {
                self.reverse.set(matches!(direction, Direction::Backwards));
            }
        }
    }

    pub fn set_loop_enabled(&self, enabled: bool) {
        // Set atomic immediately so readers see the change right away,
        // then sync the FSM on the next audio callback.
        self.loop_enabled.set(enabled);
        self.send_command(TransportEvent::SetLoopEnabled(enabled));
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

    /// Get shared sync state (for butler thread access).
    pub fn sync_state(&self) -> &Arc<SyncState> {
        &self.sync_state
    }

    /// Set sync source (Internal, MTC, MIDI Clock, LTC).
    pub fn set_sync_source(&self, source: SyncSource) {
        self.sync_state.set_source(source);
    }

    /// Get current sync source.
    pub fn get_sync_source(&self) -> SyncSource {
        self.sync_state.source()
    }

    /// Get sync status snapshot for UI display.
    pub fn sync_snapshot(&self) -> SyncSnapshot {
        self.sync_state.snapshot()
    }

    /// Check if transport is slaved to external source.
    /// Returns true only when external, following, AND locked.
    pub fn is_slaved(&self) -> bool {
        self.sync_state.is_external()
            && self.sync_state.is_following()
            && self.sync_state.is_locked()
    }

    /// Receive external position update (call when receiving MTC/LTC/MIDI Clock).
    /// Updates sync state and optionally chases to external position.
    pub fn receive_external_position(&self, beats: f64) {
        self.sync_state.set_external_position(beats);

        // If following external source and locked, update transport position
        if self.sync_state.is_following() && self.sync_state.is_locked() {
            // Apply offset if configured
            let offset_samples = self.sync_state.offset_samples();
            let offset_beats = if offset_samples != 0.0 {
                offset_samples / self.samples_per_beat()
            } else {
                0.0
            };
            self.current_beat.set(beats + offset_beats);
        }
    }

    /// Receive external tempo update (from MIDI Clock tempo detection).
    pub fn receive_external_tempo(&self, bpm: f32) {
        self.sync_state.set_external_tempo(bpm);

        // If following external tempo, update transport tempo
        if self.sync_state.is_following() && self.sync_state.is_locked() {
            self.tempo.set(bpm);
        }
    }

    /// Set sync offset in samples (positive = delay internal, negative = advance).
    pub fn set_sync_offset(&self, samples: f64) {
        self.sync_state.set_offset_samples(samples);
    }

    /// Enable/disable following external position.
    pub fn set_following(&self, follow: bool) {
        self.sync_state.set_following(follow);
    }

    /// Set SMPTE frame rate for MTC/LTC.
    pub fn set_smpte_frame_rate(&self, rate: super::sync::SmpteFrameRate) {
        self.sync_state.set_smpte_frame_rate(rate);
    }
}

impl Default for TransportManager {
    fn default() -> Self {
        Self::new(44100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::super::sync::SyncStatus;
    use super::*;

    #[test]
    fn test_new_default_state() {
        let manager = TransportManager::new(48000.0);

        assert_eq!(manager.get_tempo(), 120.0);
        assert!(manager.is_paused());
        assert_eq!(manager.get_current_beat(), 0.0);
        assert!(!manager.is_loop_enabled());
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

        assert!(manager.is_paused());

        manager.set_paused(false);
        assert!(!manager.is_paused());

        manager.set_paused(true);
        assert!(manager.is_paused());
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
        assert!(!manager.is_loop_enabled());
        assert_eq!(manager.get_loop_range(), None);

        manager.set_loop_range(2.0, 8.0);
        manager.set_loop_enabled(true);
        manager.process_commands(); // flush FSM queue

        assert!(manager.is_loop_enabled());
        assert_eq!(manager.get_loop_range(), Some((2.0, 8.0)));

        // Disable loop
        manager.set_loop_enabled(false);
        manager.process_commands();
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
        use crate::compat::{Arc, Vec};
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

    #[test]
    fn test_sync_default_state() {
        let manager = TransportManager::new(48000.0);

        assert_eq!(manager.get_sync_source(), SyncSource::Internal);
        assert!(!manager.is_slaved());

        let snap = manager.sync_snapshot();
        assert_eq!(snap.source, SyncSource::Internal);
        assert_eq!(snap.status, SyncStatus::Unlocked);
        assert!(!snap.following);
    }

    #[test]
    fn test_sync_source_change() {
        let manager = TransportManager::new(48000.0);

        manager.set_sync_source(SyncSource::MidiTimecode);
        assert_eq!(manager.get_sync_source(), SyncSource::MidiTimecode);

        let snap = manager.sync_snapshot();
        assert_eq!(snap.source, SyncSource::MidiTimecode);
        // Status should be Locking when switching to external
        assert_eq!(snap.status, SyncStatus::Locking);

        // Switch back to internal
        manager.set_sync_source(SyncSource::Internal);
        assert_eq!(manager.get_sync_source(), SyncSource::Internal);
        let snap = manager.sync_snapshot();
        assert_eq!(snap.status, SyncStatus::Unlocked);
    }

    #[test]
    fn test_external_position_following() {
        let manager = TransportManager::new(48000.0);

        // Set up external sync
        manager.set_sync_source(SyncSource::MidiClock);
        manager.sync_state().set_status(SyncStatus::Locked);
        manager.set_following(true);

        assert!(manager.is_slaved());

        // Receive external position
        manager.receive_external_position(16.0);

        // Position should be updated
        assert!((manager.get_current_beat() - 16.0).abs() < 0.001);
    }

    #[test]
    fn test_external_tempo_following() {
        let manager = TransportManager::new(48000.0);

        // Set up external sync
        manager.set_sync_source(SyncSource::MidiClock);
        manager.sync_state().set_status(SyncStatus::Locked);
        manager.set_following(true);

        // Receive external tempo
        manager.receive_external_tempo(140.0);

        // Tempo should be updated
        assert!((manager.get_tempo() - 140.0).abs() < 0.001);
    }

    #[test]
    fn test_sync_offset() {
        let manager = TransportManager::new(48000.0);

        // Set up external sync with offset
        manager.set_sync_source(SyncSource::MidiTimecode);
        manager.sync_state().set_status(SyncStatus::Locked);
        manager.set_following(true);

        // Set offset of 24000 samples (1 beat at 120 BPM, 48kHz)
        manager.set_sync_offset(24000.0);

        // Receive external position
        manager.receive_external_position(8.0);

        // Position should include offset: 8.0 + 1.0 = 9.0 beats
        assert!((manager.get_current_beat() - 9.0).abs() < 0.001);
    }

    #[test]
    fn test_not_following_when_unlocked() {
        let manager = TransportManager::new(48000.0);

        // Set external source but don't lock
        manager.set_sync_source(SyncSource::MidiClock);
        manager.set_following(true);

        // Not slaved because status is Locking, not Locked
        assert!(!manager.is_slaved());

        // Receive external position
        manager.receive_external_position(32.0);

        // Position should NOT be updated (still at 0)
        assert!((manager.get_current_beat() - 0.0).abs() < 0.001);
    }
}
