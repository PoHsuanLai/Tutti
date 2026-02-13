//! Fluent API handles for transport and metronome control

use super::click::{ClickState, MetronomeMode};
use super::{MotionState, TransportManager};
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
///     .always();
/// ```
pub struct MetronomeHandle {
    state: Arc<ClickState>,
}

impl MetronomeHandle {
    pub(crate) fn new(state: Arc<ClickState>) -> Self {
        Self { state }
    }

    /// Set metronome volume (0.0 to 1.0).
    pub fn volume(self, volume: f32) -> Self {
        self.state.set_volume(volume);
        self
    }

    /// Get metronome volume.
    pub fn get_volume(&self) -> f32 {
        self.state.volume()
    }

    /// Set accent pattern (accent every N beats).
    pub fn accent_every(self, beats: u32) -> Self {
        self.state.set_accent_every(beats);
        self
    }

    /// Get accent pattern.
    pub fn get_accent_every(&self) -> u32 {
        self.state.accent_every()
    }

    /// Set metronome mode.
    pub fn mode(self, mode: MetronomeMode) -> Self {
        self.state.set_mode(mode);
        self
    }

    /// Get metronome mode.
    pub fn get_mode(&self) -> MetronomeMode {
        self.state.mode()
    }

    /// Turn metronome off.
    pub fn off(self) -> Self {
        self.state.set_mode(MetronomeMode::Off);
        self
    }

    /// Enable metronome always.
    pub fn always(self) -> Self {
        self.state.set_mode(MetronomeMode::Always);
        self
    }

    /// Enable metronome for recording only.
    pub fn recording_only(self) -> Self {
        self.state.set_mode(MetronomeMode::RecordingOnly);
        self
    }

    /// Enable metronome for preroll only.
    pub fn preroll_only(self) -> Self {
        self.state.set_mode(MetronomeMode::PrerollOnly);
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
#[derive(Clone)]
pub struct TransportHandle {
    transport: Arc<TransportManager>,
    click_state: Arc<ClickState>,
}

impl TransportHandle {
    pub(crate) fn new(transport: Arc<TransportManager>, click_state: Arc<ClickState>) -> Self {
        Self {
            transport,
            click_state,
        }
    }

    /// Set tempo in BPM.
    pub fn tempo(self, bpm: f32) -> Self {
        self.transport.set_tempo(bpm);
        self
    }

    /// Get current tempo.
    pub fn get_tempo(&self) -> f32 {
        self.transport.get_tempo()
    }

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

    /// Seek to position in beats.
    pub fn seek(self, beats: f64) -> Self {
        self.transport.locate(beats);
        self
    }

    /// Seek to position and start playing.
    pub fn seek_and_play(self, beats: f64) -> Self {
        self.transport.locate_and_play(beats);
        self
    }

    /// Seek with smooth declick.
    pub fn seek_with_declick(self, beats: f64) -> Self {
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

    /// Get current beat position.
    pub fn current_beat(&self) -> f64 {
        self.transport.get_current_beat()
    }

    /// Set current beat position (use `locate()` for FSM-based seeking).
    pub fn set_current_beat(self, beat: f64) -> Self {
        self.transport.set_current_beat(beat);
        self
    }

    /// Check if transport is playing.
    pub fn is_playing(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Rolling)
    }

    /// Check if transport is stopped.
    pub fn is_stopped(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Stopped)
    }

    /// Check if transport is seeking.
    pub fn is_seeking(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::DeclickToLocate)
    }

    /// Check if transport is stopping.
    pub fn is_stopping(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::DeclickToStop)
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

    /// Check if recording is active.
    pub fn is_recording(&self) -> bool {
        self.transport.is_recording()
    }

    /// Start recording.
    pub fn record(self) -> Self {
        self.transport.set_recording(true);
        self
    }

    /// Stop recording.
    pub fn stop_recording(self) -> Self {
        self.transport.set_recording(false);
        self
    }

    /// Check if in preroll count-in.
    pub fn is_in_preroll(&self) -> bool {
        self.transport.is_in_preroll()
    }

    /// Start preroll count-in.
    pub fn start_preroll(self) -> Self {
        self.transport.set_in_preroll(true);
        self
    }

    /// End preroll count-in.
    pub fn end_preroll(self) -> Self {
        self.transport.set_in_preroll(false);
        self
    }

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
        MetronomeHandle::new(self.click_state.clone())
    }

    /// Get reference to underlying TransportManager for direct access.
    pub fn manager(&self) -> &Arc<TransportManager> {
        &self.transport
    }

    /// Get reference to the click state for creating ClickNode.
    pub fn click_state(&self) -> &Arc<ClickState> {
        &self.click_state
    }
}

impl super::TransportReader for TransportHandle {
    fn current_beat(&self) -> f64 {
        self.transport.get_current_beat()
    }

    fn is_loop_enabled(&self) -> bool {
        self.transport.is_loop_enabled()
    }

    fn get_loop_range(&self) -> Option<(f64, f64)> {
        self.transport.get_loop_range()
    }

    fn is_playing(&self) -> bool {
        matches!(self.transport.motion_state(), MotionState::Rolling)
    }

    fn is_recording(&self) -> bool {
        self.transport.is_recording()
    }

    fn is_in_preroll(&self) -> bool {
        self.transport.is_in_preroll()
    }

    fn tempo(&self) -> f32 {
        self.transport.get_tempo()
    }
}
