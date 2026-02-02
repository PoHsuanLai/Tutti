//! Fluent API handles for transport and metronome control

use super::{Metronome, MetronomeMode, MotionState, TransportManager};
use crate::compat::Arc;

/// Fluent API handle for metronome control.
///
/// Created via `transport.metronome()`.
///
/// # Example
/// ```ignore
/// engine.transport()
///     .metronome()
///     .volume(0.7)
///     .accent_every(4)
///     .mode(MetronomeMode::Always);
/// ```
pub struct MetronomeHandle {
    metronome: Arc<Metronome>,
}

impl MetronomeHandle {
    pub(crate) fn new(metronome: Arc<Metronome>) -> Self {
        Self { metronome }
    }

    /// Set metronome volume (0.0 to 1.0).
    pub fn volume(self, volume: f32) -> Self {
        self.metronome.set_volume(volume);
        self
    }

    /// Get metronome volume.
    pub fn get_volume(&self) -> f32 {
        self.metronome.volume()
    }

    /// Set accent pattern (accent every N beats).
    pub fn accent_every(self, beats: u32) -> Self {
        self.metronome.set_accent_every(beats);
        self
    }

    /// Get accent pattern.
    pub fn get_accent_every(&self) -> u32 {
        self.metronome.accent_every()
    }

    /// Set metronome mode.
    pub fn mode(self, mode: MetronomeMode) -> Self {
        self.metronome.set_mode(mode);
        self
    }

    /// Get metronome mode.
    pub fn get_mode(&self) -> MetronomeMode {
        self.metronome.mode()
    }

    /// Turn metronome off.
    pub fn off(self) -> Self {
        self.metronome.set_mode(MetronomeMode::Off);
        self
    }

    /// Enable metronome always.
    pub fn always(self) -> Self {
        self.metronome.set_mode(MetronomeMode::Always);
        self
    }

    /// Enable metronome for recording only.
    pub fn recording_only(self) -> Self {
        self.metronome.set_mode(MetronomeMode::RecordingOnly);
        self
    }

    /// Enable metronome for preroll only.
    pub fn preroll_only(self) -> Self {
        self.metronome.set_mode(MetronomeMode::PrerollOnly);
        self
    }
}

/// Fluent API handle for transport control.
///
/// Created via `engine.transport()`.
///
/// # Example
/// ```ignore
/// engine.transport()
///     .tempo(128.0)
///     .loop_range(0.0, 16.0)
///     .enable_loop()
///     .metronome()
///         .volume(0.7)
///         .accent_every(4)
///         .always()
///     .play();
/// ```
pub struct TransportHandle {
    transport: Arc<TransportManager>,
    metronome: Arc<Metronome>,
}

impl TransportHandle {
    pub(crate) fn new(transport: Arc<TransportManager>, metronome: Arc<Metronome>) -> Self {
        Self {
            transport,
            metronome,
        }
    }

    // ===== Tempo control =====

    /// Set tempo in BPM.
    pub fn tempo(self, bpm: f32) -> Self {
        self.transport.set_tempo(bpm);
        self
    }

    /// Get current tempo.
    pub fn get_tempo(&self) -> f32 {
        self.transport.get_tempo()
    }

    // ===== Transport control =====

    /// Start playback.
    pub fn play(self) -> Self {
        self.transport.play();
        self
    }

    /// Stop playback with declick.
    pub fn stop(self) -> Self {
        self.transport.stop();
        self
    }

    /// Stop playback immediately (no declick).
    pub fn stop_immediate(self) -> Self {
        self.transport.stop_immediate();
        self
    }

    /// Locate to position in beats.
    pub fn locate(self, beats: f64) -> Self {
        self.transport.locate(beats);
        self
    }

    /// Locate to position and start playing.
    pub fn locate_and_play(self, beats: f64) -> Self {
        self.transport.locate_and_play(beats);
        self
    }

    /// Locate with smooth declick.
    pub fn locate_with_declick(self, beats: f64) -> Self {
        self.transport.locate_with_declick(beats);
        self
    }

    /// Start fast forward.
    pub fn fast_forward(self) -> Self {
        self.transport.fast_forward();
        self
    }

    /// Start rewind.
    pub fn rewind(self) -> Self {
        self.transport.rewind();
        self
    }

    /// End scrub/shuttle mode.
    pub fn end_scrub(self) -> Self {
        self.transport.end_scrub();
        self
    }

    /// Toggle reverse playback.
    pub fn reverse(self) -> Self {
        self.transport.reverse();
        self
    }

    // ===== Loop control =====

    /// Set loop range (start and end in beats).
    pub fn loop_range(self, start: f64, end: f64) -> Self {
        self.transport.set_loop_range_fsm(start, end);
        self
    }

    /// Enable loop.
    pub fn enable_loop(self) -> Self {
        self.transport.set_loop_enabled(true);
        self
    }

    /// Disable loop.
    pub fn disable_loop(self) -> Self {
        self.transport.set_loop_enabled(false);
        self
    }

    /// Toggle loop enabled/disabled.
    pub fn toggle_loop(self) -> Self {
        self.transport.toggle_loop();
        self
    }

    /// Clear loop range.
    pub fn clear_loop(self) -> Self {
        self.transport.clear_loop();
        self
    }

    /// Get loop range if enabled.
    pub fn get_loop_range(&self) -> Option<(f64, f64)> {
        self.transport.get_loop_range()
    }

    /// Check if loop is enabled.
    pub fn is_loop_enabled(&self) -> bool {
        self.transport.is_loop_enabled()
    }

    // ===== Position queries =====

    /// Get current beat position.
    pub fn current_beat(&self) -> f64 {
        self.transport.get_current_beat()
    }

    /// Set current beat position (use `locate()` for FSM-based seeking).
    pub fn set_current_beat(self, beat: f64) -> Self {
        self.transport.set_current_beat(beat);
        self
    }

    // ===== State queries =====

    /// Check if transport is playing.
    pub fn is_playing(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Rolling)
    }

    /// Check if transport is stopped.
    pub fn is_stopped(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Stopped)
    }

    /// Check if transport is seeking/locating.
    pub fn is_seeking(&self) -> bool {
        matches!(
            self.transport.motion_state(),
            MotionState::DeclickToLocate
        )
    }

    /// Check if transport is stopping.
    pub fn is_stopping(&self) -> bool {
        matches!(
            self.transport.motion_state(),
            MotionState::DeclickToStop
        )
    }

    /// Check if transport is in fast forward mode.
    pub fn is_fast_forwarding(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::FastForward)
    }

    /// Check if transport is in rewind mode.
    pub fn is_rewinding(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Rewind)
    }

    /// Get current motion state.
    pub fn motion_state(&self) -> MotionState {
        self.transport.motion_state()
    }

    /// Check if paused (deprecated - use `is_stopped()` or `motion_state()`).
    #[deprecated(since = "0.1.0", note = "Use is_stopped() or motion_state() instead")]
    pub fn is_paused(&self) -> bool {
        self.transport.is_paused()
    }

    // ===== Metronome access =====

    /// Get metronome handle for fluent configuration.
    ///
    /// # Example
    /// ```ignore
    /// transport.metronome()
    ///     .volume(0.8)
    ///     .accent_every(4)
    ///     .always();
    /// ```
    pub fn metronome(&self) -> MetronomeHandle {
        MetronomeHandle::new(self.metronome.clone())
    }

    // ===== Access to underlying managers (advanced use) =====

    /// Get reference to underlying TransportManager for direct access.
    pub fn manager(&self) -> &Arc<TransportManager> {
        &self.transport
    }
}
