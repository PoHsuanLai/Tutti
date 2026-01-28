//! Automation manager for parameter automation lanes.

use super::{AutomationLane, AutomationRecordingConfig, AutomationTarget};
use audio_automation::{AutomationEnvelope, AutomationPoint, AutomationState};
use dashmap::DashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// Automation lane manager.
#[derive(Debug)]
pub struct AutomationManager {
    lanes: DashMap<AutomationTarget, AutomationLane>,
    enabled: AtomicBool,
    default_config: AutomationRecordingConfig,
}

impl AutomationManager {
    /// Create a new automation manager
    pub fn new() -> Self {
        Self {
            lanes: DashMap::new(),
            enabled: AtomicBool::new(true),
            default_config: AutomationRecordingConfig::default(),
        }
    }

    /// Create with custom default recording configuration
    pub fn with_config(config: AutomationRecordingConfig) -> Self {
        Self {
            lanes: DashMap::new(),
            enabled: AtomicBool::new(true),
            default_config: config,
        }
    }

    // =========================================================================
    // Global Control
    // =========================================================================

    /// Enable or disable all automation
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Check if automation is globally enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Set the default recording configuration for new lanes
    pub fn set_default_config(&mut self, config: AutomationRecordingConfig) {
        self.default_config = config;
    }

    // =========================================================================
    // Lane Management
    // =========================================================================

    /// Get or create a lane for the given target
    ///
    /// If the lane doesn't exist, creates a new one with default settings.
    pub fn get_or_create_lane(
        &self,
        target: AutomationTarget,
    ) -> dashmap::mapref::one::RefMut<'_, AutomationTarget, AutomationLane> {
        self.lanes.entry(target.clone()).or_insert_with(|| {
            let mut lane = AutomationLane::new(target);
            lane.set_config(self.default_config);
            lane
        })
    }

    /// Get an existing lane (returns None if not found)
    pub fn get_lane(
        &self,
        target: &AutomationTarget,
    ) -> Option<dashmap::mapref::one::Ref<'_, AutomationTarget, AutomationLane>> {
        self.lanes.get(target)
    }

    /// Get a mutable reference to an existing lane
    pub fn get_lane_mut(
        &self,
        target: &AutomationTarget,
    ) -> Option<dashmap::mapref::one::RefMut<'_, AutomationTarget, AutomationLane>> {
        self.lanes.get_mut(target)
    }

    /// Create a new lane with custom configuration
    pub fn create_lane(
        &self,
        target: AutomationTarget,
        config: AutomationRecordingConfig,
    ) -> dashmap::mapref::one::RefMut<'_, AutomationTarget, AutomationLane> {
        self.lanes.entry(target.clone()).or_insert_with(|| {
            let mut lane = AutomationLane::new(target);
            lane.set_config(config);
            lane
        })
    }

    /// Create a lane with an existing envelope
    pub fn create_lane_with_envelope(&self, envelope: AutomationEnvelope<AutomationTarget>) {
        let target = envelope.target.clone();
        let lane = AutomationLane::with_envelope(envelope);
        self.lanes.insert(target, lane);
    }

    /// Remove a lane
    pub fn remove_lane(
        &self,
        target: &AutomationTarget,
    ) -> Option<(AutomationTarget, AutomationLane)> {
        self.lanes.remove(target)
    }

    /// Check if a lane exists
    pub fn has_lane(&self, target: &AutomationTarget) -> bool {
        self.lanes.contains_key(target)
    }

    /// Get the number of lanes
    pub fn lane_count(&self) -> usize {
        self.lanes.len()
    }

    /// Get all targets (for iteration)
    pub fn targets(&self) -> Vec<AutomationTarget> {
        self.lanes.iter().map(|r| r.key().clone()).collect()
    }

    /// Clear all lanes
    pub fn clear(&self) {
        self.lanes.clear();
    }

    // =========================================================================
    // Value Access (Real-time Safe)
    // =========================================================================

    /// Get the current value for a target at the given beat position
    ///
    /// Returns None if automation is disabled or the lane doesn't exist.
    /// This is the primary method for the audio thread to read automation.
    #[inline]
    pub fn get_value(&self, target: &AutomationTarget, beat: f64) -> Option<f32> {
        if !self.is_enabled() {
            return None;
        }

        self.lanes.get(target).map(|lane| lane.get_value_at(beat))
    }

    /// Get multiple values at once (more efficient for bulk reads)
    pub fn get_values(&self, targets: &[AutomationTarget], beat: f64) -> Vec<Option<f32>> {
        if !self.is_enabled() {
            return vec![None; targets.len()];
        }

        targets
            .iter()
            .map(|target| self.lanes.get(target).map(|lane| lane.get_value_at(beat)))
            .collect()
    }

    // =========================================================================
    // State Control
    // =========================================================================

    /// Set the automation state for a specific target
    pub fn set_state(&self, target: &AutomationTarget, state: AutomationState) {
        if let Some(mut lane) = self.lanes.get_mut(target) {
            lane.set_state(state);
        }
    }

    /// Get the automation state for a specific target
    pub fn get_state(&self, target: &AutomationTarget) -> Option<AutomationState> {
        self.lanes.get(target).map(|lane| lane.state())
    }

    /// Set all lanes to the same state
    pub fn set_all_states(&self, state: AutomationState) {
        for mut lane_ref in self.lanes.iter_mut() {
            lane_ref.set_state(state);
        }
    }

    /// Set state for all lanes matching a predicate
    pub fn set_states_where<F>(&self, state: AutomationState, predicate: F)
    where
        F: Fn(&AutomationTarget) -> bool,
    {
        for mut lane_ref in self.lanes.iter_mut() {
            if predicate(lane_ref.key()) {
                lane_ref.set_state(state);
            }
        }
    }

    // =========================================================================
    // Recording Control
    // =========================================================================

    /// Signal that a control has been "touched" (user started interacting)
    pub fn touch(&self, target: &AutomationTarget, beat: f64, value: f32) {
        if let Some(mut lane) = self.lanes.get_mut(target) {
            lane.touch(beat, value);
        }
    }

    /// Record a value for a target
    pub fn record(&self, target: &AutomationTarget, beat: f64, value: f32) {
        if let Some(mut lane) = self.lanes.get_mut(target) {
            lane.record(beat, value);
        }
    }

    /// Signal that a control has been "released" (user stopped interacting)
    pub fn release(&self, target: &AutomationTarget, beat: f64, value: f32) {
        if let Some(mut lane) = self.lanes.get_mut(target) {
            lane.release(beat, value);
        }
    }

    /// Batch recording: record the same beat for multiple targets
    pub fn record_batch(&self, beat: f64, values: &[(AutomationTarget, f32)]) {
        for (target, value) in values {
            self.record(target, beat, *value);
        }
    }

    // =========================================================================
    // Envelope Manipulation
    // =========================================================================

    /// Add a point to a specific lane
    pub fn add_point(&self, target: &AutomationTarget, point: AutomationPoint) {
        if let Some(lane) = self.lanes.get(target) {
            lane.add_point(point);
        }
    }

    /// Remove a point from a specific lane
    pub fn remove_point_at(&self, target: &AutomationTarget, beat: f64) -> Option<AutomationPoint> {
        self.lanes
            .get(target)
            .and_then(|lane| lane.remove_point_at(beat))
    }

    /// Clear all points from a specific lane
    pub fn clear_lane(&self, target: &AutomationTarget) {
        if let Some(lane) = self.lanes.get(target) {
            lane.clear();
        }
    }

    /// Simplify a specific lane
    pub fn simplify_lane(&self, target: &AutomationTarget, tolerance: f32) {
        if let Some(lane) = self.lanes.get(target) {
            lane.simplify(tolerance);
        }
    }

    /// Simplify all lanes
    pub fn simplify_all(&self, tolerance: f32) {
        for lane_ref in self.lanes.iter() {
            lane_ref.simplify(tolerance);
        }
    }

    // =========================================================================
    // Bulk Operations
    // =========================================================================

    /// Get all lanes for a specific node (by node_id)
    pub fn lanes_for_node(&self, node_id: u64) -> Vec<AutomationTarget> {
        self.lanes
            .iter()
            .filter_map(|r| {
                if r.key().node_id() == Some(node_id) {
                    Some(r.key().clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Remove all lanes for a specific node
    pub fn remove_lanes_for_node(&self, node_id: u64) {
        let targets: Vec<_> = self.lanes_for_node(node_id);
        for target in targets {
            self.lanes.remove(&target);
        }
    }

    /// Get all master control lanes
    pub fn master_lanes(&self) -> Vec<AutomationTarget> {
        self.lanes
            .iter()
            .filter_map(|r| {
                if r.key().is_master() {
                    Some(r.key().clone())
                } else {
                    None
                }
            })
            .collect()
    }

    // =========================================================================
    // Snapshot/Restore
    // =========================================================================

    /// Create a snapshot of all lanes (for undo/redo)
    pub fn snapshot(&self) -> AutomationSnapshot {
        let lanes: Vec<_> = self
            .lanes
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect();

        AutomationSnapshot {
            lanes,
            enabled: self.is_enabled(),
        }
    }

    /// Restore from a snapshot
    pub fn restore(&self, snapshot: &AutomationSnapshot) {
        self.lanes.clear();
        for (target, lane) in &snapshot.lanes {
            self.lanes.insert(target.clone(), lane.clone());
        }
        self.set_enabled(snapshot.enabled);
    }
}

impl Default for AutomationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for AutomationManager {
    fn clone(&self) -> Self {
        let new_manager = Self::new();
        for lane_ref in self.lanes.iter() {
            new_manager
                .lanes
                .insert(lane_ref.key().clone(), lane_ref.value().clone());
        }
        new_manager.set_enabled(self.is_enabled());
        new_manager
    }
}

/// Snapshot of automation state (for undo/redo)
#[derive(Debug, Clone)]
pub struct AutomationSnapshot {
    /// All lanes and their data
    pub lanes: Vec<(AutomationTarget, AutomationLane)>,
    /// Global enabled state
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_create_manager() {
        let manager = AutomationManager::new();
        assert!(manager.is_enabled());
        assert_eq!(manager.lane_count(), 0);
    }

    #[test]
    fn test_get_or_create_lane() {
        let manager = AutomationManager::new();
        let target = AutomationTarget::MasterVolume;

        {
            let _lane = manager.get_or_create_lane(target.clone());
        }

        assert!(manager.has_lane(&target));
        assert_eq!(manager.lane_count(), 1);
    }

    #[test]
    fn test_get_value() {
        let manager = AutomationManager::new();
        let target = AutomationTarget::MasterVolume;

        // Create lane and add points
        {
            let lane = manager.get_or_create_lane(target.clone());
            lane.add_point(AutomationPoint::new(0.0, 0.0));
            lane.add_point(AutomationPoint::new(4.0, 1.0));
            drop(lane);
        }

        // Set to Play mode
        manager.set_state(&target, AutomationState::Play);

        // Read values
        let val = manager.get_value(&target, 2.0);
        assert!(val.is_some());
        assert!((val.unwrap() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_global_disable() {
        let manager = AutomationManager::new();
        let target = AutomationTarget::MasterVolume;

        {
            let lane = manager.get_or_create_lane(target.clone());
            lane.add_point(AutomationPoint::new(0.0, 0.5));
            drop(lane);
        }

        manager.set_state(&target, AutomationState::Play);

        // Should return value when enabled
        assert!(manager.get_value(&target, 0.0).is_some());

        // Disable automation
        manager.set_enabled(false);

        // Should return None when disabled
        assert!(manager.get_value(&target, 0.0).is_none());
    }

    #[test]
    fn test_recording() {
        let manager = AutomationManager::new();
        let target = AutomationTarget::MasterVolume;

        {
            let mut lane = manager.get_or_create_lane(target.clone());
            lane.set_state(AutomationState::Write);
        }

        // Record some values
        manager.record(&target, 0.0, 0.1);
        manager.record(&target, 1.0, 0.2);
        manager.record(&target, 2.0, 0.3);

        // Check lane has points
        let lane = manager.get_lane(&target).unwrap();
        assert!(lane.len() >= 3);
    }

    #[test]
    fn test_lanes_for_node() {
        let manager = AutomationManager::new();

        // Create several lanes for node 42
        manager.get_or_create_lane(AutomationTarget::node_param(42, 0));
        manager.get_or_create_lane(AutomationTarget::node_param(42, 1));
        manager.get_or_create_lane(AutomationTarget::node_param(42, 2));
        // And one for node 43
        manager.get_or_create_lane(AutomationTarget::node_param(43, 0));
        // And master
        manager.get_or_create_lane(AutomationTarget::MasterVolume);

        let node_42_lanes = manager.lanes_for_node(42);
        assert_eq!(node_42_lanes.len(), 3);

        let node_43_lanes = manager.lanes_for_node(43);
        assert_eq!(node_43_lanes.len(), 1);
    }

    #[test]
    fn test_snapshot_restore() {
        let manager = AutomationManager::new();
        let target = AutomationTarget::MasterVolume;

        {
            let lane = manager.get_or_create_lane(target.clone());
            lane.add_point(AutomationPoint::new(0.0, 0.5));
            lane.add_point(AutomationPoint::new(4.0, 1.0));
        }

        // Take snapshot
        let snapshot = manager.snapshot();

        // Clear manager
        manager.clear();
        assert_eq!(manager.lane_count(), 0);

        // Restore
        manager.restore(&snapshot);
        assert_eq!(manager.lane_count(), 1);
        assert!(manager.has_lane(&target));
    }

    #[test]
    fn test_set_all_states() {
        let manager = AutomationManager::new();

        manager.get_or_create_lane(AutomationTarget::MasterVolume);
        manager.get_or_create_lane(AutomationTarget::MasterPan);
        manager.get_or_create_lane(AutomationTarget::Tempo);

        // Set all to Play
        manager.set_all_states(AutomationState::Play);

        for target in manager.targets() {
            assert_eq!(manager.get_state(&target), Some(AutomationState::Play));
        }
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;

        let manager = Arc::new(AutomationManager::new());
        let target = AutomationTarget::MasterVolume;

        // Create the lane and set to Write mode (always records)
        manager.get_or_create_lane(target.clone());
        manager.set_state(&target, AutomationState::Write);

        let manager1 = Arc::clone(&manager);
        let target1 = target.clone();
        let t1 = thread::spawn(move || {
            for i in 0..100 {
                manager1.record(&target1, i as f64 * 0.01, i as f32 * 0.01);
            }
        });

        let manager2 = Arc::clone(&manager);
        let target2 = target.clone();
        let t2 = thread::spawn(move || {
            for i in 0..100 {
                let _ = manager2.get_value(&target2, i as f64 * 0.01);
            }
        });

        t1.join().unwrap();
        t2.join().unwrap();

        // Should have recorded points without panicking
        let lane = manager.get_lane(&target).unwrap();
        assert!(!lane.is_empty());
    }
}
