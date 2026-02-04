//! Automation lane with recording state management.

use super::AutomationTarget;
use audio_automation::{AutomationEnvelope, AutomationPoint, AutomationState, CurveType};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Automation point recording configuration.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AutomationRecordingConfig {
    pub min_point_interval: f64,
    pub simplify_tolerance: f32,
    pub auto_simplify: bool,
    pub default_curve: CurveType,
}

impl Default for AutomationRecordingConfig {
    fn default() -> Self {
        Self {
            min_point_interval: 0.01, // ~100 points per beat max
            simplify_tolerance: 0.01, // 1% tolerance
            auto_simplify: true,
            default_curve: CurveType::Linear,
        }
    }
}

/// Recording session state (internal).
#[derive(Debug, Clone)]
struct RecordingSession {
    last_recorded_beat: f64,
    last_value: f32,
    is_touching: bool,
}

/// Automation lane for one parameter.
#[derive(Debug)]
pub struct AutomationLane {
    envelope: Arc<RwLock<AutomationEnvelope<AutomationTarget>>>,
    state: AutomationState,
    config: AutomationRecordingConfig,
    recording_session: Option<RecordingSession>,
    manual_value: f32,
}

impl AutomationLane {
    /// Create a new automation lane for the given target
    pub fn new(target: AutomationTarget) -> Self {
        let (min, max, default) = target.default_range();
        let envelope = AutomationEnvelope::new(target).with_range(min, max);

        Self {
            envelope: Arc::new(RwLock::new(envelope)),
            state: AutomationState::Off,
            config: AutomationRecordingConfig::default(),
            recording_session: None,
            manual_value: default,
        }
    }

    /// Create a new automation lane with an existing envelope
    pub fn with_envelope(envelope: AutomationEnvelope<AutomationTarget>) -> Self {
        let manual_value = envelope.get_value_at(0.0).unwrap_or(0.5);
        Self {
            envelope: Arc::new(RwLock::new(envelope)),
            state: AutomationState::Off,
            config: AutomationRecordingConfig::default(),
            recording_session: None,
            manual_value,
        }
    }

    /// Get a shared reference to the envelope (for read-only access)
    pub fn envelope(&self) -> Arc<RwLock<AutomationEnvelope<AutomationTarget>>> {
        Arc::clone(&self.envelope)
    }

    /// Get the current automation state
    pub fn state(&self) -> AutomationState {
        self.state
    }

    /// Set the automation state
    ///
    /// If changing to Write mode while transport is playing, starts recording.
    /// If changing away from Write mode, stops recording.
    pub fn set_state(&mut self, state: AutomationState) {
        // Handle state transitions
        if self.state != state {
            // Stopping recording
            if self.state.can_record() && !state.can_record() {
                self.stop_recording();
            }
            self.state = state;
        }
    }

    /// Get the current recording configuration
    pub fn config(&self) -> &AutomationRecordingConfig {
        &self.config
    }

    /// Get mutable access to the recording configuration
    pub fn config_mut(&mut self) -> &mut AutomationRecordingConfig {
        &mut self.config
    }

    /// Set the recording configuration
    pub fn set_config(&mut self, config: AutomationRecordingConfig) {
        self.config = config;
    }

    /// Get the manual value (used when state is Off)
    pub fn manual_value(&self) -> f32 {
        self.manual_value
    }

    /// Set the manual value
    pub fn set_manual_value(&mut self, value: f32) {
        self.manual_value = value;
    }

    /// Get the current value at the given beat position
    ///
    /// Takes into account the current state:
    /// - Off: Returns manual_value
    /// - Play/Touch/Latch: Returns envelope value (or manual if empty)
    /// - Write: Returns last recorded value or manual
    pub fn get_value_at(&self, beat: f64) -> f32 {
        match self.state {
            AutomationState::Off => self.manual_value,
            AutomationState::Write => {
                // During write, return last recorded value or manual
                self.recording_session
                    .as_ref()
                    .map(|s| s.last_value)
                    .unwrap_or(self.manual_value)
            }
            AutomationState::Play | AutomationState::Touch | AutomationState::Latch => {
                // Check if in latch continuation (touched then released)
                if self.state == AutomationState::Latch {
                    if let Some(ref session) = self.recording_session {
                        if !session.is_touching {
                            // Latch mode: continue at last value
                            return session.last_value;
                        }
                    }
                }

                // Normal playback: read from envelope
                self.envelope
                    .read()
                    .get_value_at(beat)
                    .unwrap_or(self.manual_value)
            }
        }
    }

    /// Called when a control is "touched" (user starts interacting)
    ///
    /// For Touch and Latch modes, this starts recording.
    pub fn touch(&mut self, beat: f64, value: f32) {
        if !self.state.starts_on_touch() {
            return;
        }

        // Start recording session
        self.recording_session = Some(RecordingSession {
            last_recorded_beat: beat,
            last_value: value,
            is_touching: true,
        });

        // Record the initial touch point
        self.record_point(beat, value);
    }

    /// Record a value at the given beat position
    ///
    /// Only records if in a recording state (Write, Touch, Latch) and
    /// respects the minimum point interval setting.
    pub fn record(&mut self, beat: f64, value: f32) {
        if !self.state.can_record() {
            return;
        }

        // For Write mode, always record
        // For Touch/Latch, only record if touching
        if self.state == AutomationState::Write {
            // Start session if needed
            if self.recording_session.is_none() {
                self.recording_session = Some(RecordingSession {
                    last_recorded_beat: beat,
                    last_value: value,
                    is_touching: true,
                });
            }
        } else if let Some(ref session) = self.recording_session {
            if !session.is_touching {
                return; // Touch/Latch requires active touch
            }
        } else {
            return; // No session, don't record
        }

        // Check minimum interval
        if let Some(ref session) = self.recording_session {
            let interval = beat - session.last_recorded_beat;
            if interval < self.config.min_point_interval && interval > 0.0 {
                return;
            }
        }

        // Record the point
        self.record_point(beat, value);

        // Update session
        if let Some(ref mut session) = self.recording_session {
            session.last_recorded_beat = beat;
            session.last_value = value;
        }
    }

    /// Called when a control is "released" (user stops interacting)
    ///
    /// For Touch mode, this stops recording.
    /// For Latch mode, recording continues at the last value.
    pub fn release(&mut self, beat: f64, value: f32) {
        if let Some(ref mut session) = self.recording_session {
            session.is_touching = false;
            session.last_value = value;

            if self.state.stops_on_release() {
                // Touch mode: stop recording and optionally simplify
                self.record_point(beat, value);
                self.stop_recording();
            }
            // Latch mode: continue at last value (handled in get_value_at)
        }
    }

    /// Stop recording and optionally simplify the envelope
    fn stop_recording(&mut self) {
        if let Some(_session) = self.recording_session.take() {
            if self.config.auto_simplify {
                self.envelope
                    .write()
                    .simplify(self.config.simplify_tolerance);
            }
        }
    }

    /// Record a single point to the envelope
    fn record_point(&self, beat: f64, value: f32) {
        let mut envelope = self.envelope.write();

        // For Write mode in certain ranges, we might want to remove existing points
        // For now, just add/replace the point
        envelope.add_point(AutomationPoint::with_curve(
            beat,
            value,
            self.config.default_curve,
        ));
    }

    /// Add a point to the envelope manually
    pub fn add_point(&self, point: AutomationPoint) {
        self.envelope.write().add_point(point);
    }

    /// Remove a point at the given beat position
    pub fn remove_point_at(&self, beat: f64) -> Option<AutomationPoint> {
        self.envelope.write().remove_point_at(beat)
    }

    /// Clear all points from the envelope
    pub fn clear(&self) {
        self.envelope.write().clear();
    }

    /// Get the number of points in the envelope
    pub fn len(&self) -> usize {
        self.envelope.read().len()
    }

    /// Check if the envelope is empty
    pub fn is_empty(&self) -> bool {
        self.envelope.read().is_empty()
    }

    /// Simplify the envelope by removing redundant points
    pub fn simplify(&self, tolerance: f32) {
        self.envelope.write().simplify(tolerance);
    }

    /// Enable/disable the envelope
    pub fn set_enabled(&self, enabled: bool) {
        self.envelope.write().enabled = enabled;
    }

    /// Check if the envelope is enabled
    pub fn is_enabled(&self) -> bool {
        self.envelope.read().enabled
    }
}

impl Clone for AutomationLane {
    fn clone(&self) -> Self {
        Self {
            envelope: Arc::new(RwLock::new(self.envelope.read().clone())),
            state: self.state,
            config: self.config,
            recording_session: self.recording_session.clone(),
            manual_value: self.manual_value,
        }
    }
}

// Serialization support (envelope only, not runtime state)
impl Serialize for AutomationLane {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Serialize only the envelope and config
        #[derive(Serialize)]
        struct LaneData<'a> {
            envelope: &'a AutomationEnvelope<AutomationTarget>,
            config: &'a AutomationRecordingConfig,
            manual_value: f32,
        }

        let envelope = self.envelope.read();
        let data = LaneData {
            envelope: &envelope,
            config: &self.config,
            manual_value: self.manual_value,
        };
        data.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AutomationLane {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct LaneData {
            envelope: AutomationEnvelope<AutomationTarget>,
            config: AutomationRecordingConfig,
            manual_value: f32,
        }

        let data = LaneData::deserialize(deserializer)?;
        Ok(Self {
            envelope: Arc::new(RwLock::new(data.envelope)),
            state: AutomationState::Off, // Always start in Off state
            config: data.config,
            recording_session: None,
            manual_value: data.manual_value,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_lane() {
        let lane = AutomationLane::new(AutomationTarget::MasterVolume);
        assert_eq!(lane.state(), AutomationState::Off);
        assert!(lane.is_empty());
        assert!((lane.manual_value() - 1.0).abs() < 0.001); // Default for volume
    }

    #[test]
    fn test_manual_value_when_off() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);
        lane.set_manual_value(0.5);

        // Should return manual value regardless of beat position
        assert!((lane.get_value_at(0.0) - 0.5).abs() < 0.001);
        assert!((lane.get_value_at(100.0) - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_playback_mode() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);

        // Add some automation points
        lane.add_point(AutomationPoint::new(0.0, 0.0));
        lane.add_point(AutomationPoint::new(4.0, 1.0));

        // In Off mode, should return manual value
        lane.set_manual_value(0.5);
        assert!((lane.get_value_at(2.0) - 0.5).abs() < 0.001);

        // In Play mode, should read from envelope
        lane.set_state(AutomationState::Play);
        assert!((lane.get_value_at(2.0) - 0.5).abs() < 0.001); // Midpoint of linear ramp
    }

    #[test]
    fn test_touch_recording() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);

        // Disable auto-simplify for predictable test behavior
        lane.config_mut().auto_simplify = false;

        lane.set_state(AutomationState::Touch);

        // Touch at beat 0 with value 0.5
        lane.touch(0.0, 0.5);
        assert_eq!(lane.len(), 1);

        // Record some values
        lane.record(1.0, 0.6);
        lane.record(2.0, 0.7);
        assert_eq!(lane.len(), 3);

        // Release
        lane.release(3.0, 0.8);
        assert_eq!(lane.len(), 4); // 4 points without simplification
    }

    #[test]
    fn test_latch_continuation() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);

        // Add initial automation
        lane.add_point(AutomationPoint::new(0.0, 0.0));
        lane.add_point(AutomationPoint::new(10.0, 1.0));

        lane.set_state(AutomationState::Latch);

        // Touch and record at beat 2
        lane.touch(2.0, 0.5);
        lane.record(3.0, 0.6);

        // Release at beat 4 with value 0.7
        lane.release(4.0, 0.7);

        // After release, should continue at last value (0.7)
        // Note: This test depends on the Latch mode implementation
        assert!((lane.get_value_at(5.0) - 0.7).abs() < 0.001);
    }

    #[test]
    fn test_write_mode() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);
        lane.set_state(AutomationState::Write);

        // In write mode, record() should always work
        lane.record(0.0, 0.1);
        lane.record(1.0, 0.2);
        lane.record(2.0, 0.3);

        assert!(lane.len() >= 3);
    }

    #[test]
    fn test_minimum_interval() {
        let mut lane = AutomationLane::new(AutomationTarget::MasterVolume);

        let config = AutomationRecordingConfig {
            min_point_interval: 0.5, // Large interval for testing
            auto_simplify: false,
            ..Default::default()
        };
        lane.set_config(config);

        lane.set_state(AutomationState::Write);

        // Record at beat 0
        lane.record(0.0, 0.5);
        // Try to record at 0.1 (too soon)
        lane.record(0.1, 0.6);
        // Try to record at 0.4 (still too soon)
        lane.record(0.4, 0.7);
        // Record at 0.5 (should work)
        lane.record(0.5, 0.8);

        // Should only have 2 points due to interval filtering
        assert_eq!(lane.len(), 2);
    }

    #[test]
    fn test_serialization() {
        let lane = AutomationLane::new(AutomationTarget::MasterVolume);
        lane.add_point(AutomationPoint::new(0.0, 0.0));
        lane.add_point(AutomationPoint::with_curve(4.0, 1.0, CurveType::SCurve));

        let json = serde_json::to_string(&lane).unwrap();
        let restored: AutomationLane = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 2);
        assert_eq!(restored.state(), AutomationState::Off); // Always starts Off
    }
}
