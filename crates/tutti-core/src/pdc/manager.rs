//! Plugin Delay Compensation (PDC) Manager.

use crate::compat::{Arc, Vec};
use crate::{AtomicBool, AtomicUsize, Ordering};
use arc_swap::ArcSwap;

#[derive(Clone)]
pub struct PdcState {
    pub channel_latencies: Vec<usize>,
    pub return_latencies: Vec<usize>,
    pub channel_compensations: Vec<usize>,
    pub return_compensations: Vec<usize>,
    pub max_latency: usize,
}

impl PdcState {
    pub fn new(channel_count: usize, return_count: usize) -> Self {
        Self {
            channel_latencies: vec![0; channel_count],
            return_latencies: vec![0; return_count],
            channel_compensations: vec![0; channel_count],
            return_compensations: vec![0; return_count],
            max_latency: 0,
        }
    }

    pub fn recalculate(&mut self) {
        let max_track = self.channel_latencies.iter().copied().max().unwrap_or(0);
        let max_return = self.return_latencies.iter().copied().max().unwrap_or(0);
        self.max_latency = max_track.max(max_return);

        for (i, latency) in self.channel_latencies.iter().enumerate() {
            self.channel_compensations[i] = self.max_latency.saturating_sub(*latency);
        }

        for (i, latency) in self.return_latencies.iter().enumerate() {
            self.return_compensations[i] = self.max_latency.saturating_sub(*latency);
        }
    }
}

pub struct PdcManager {
    state: ArcSwap<PdcState>,
    enabled: AtomicBool,
    max_allowed_latency: AtomicUsize,
}

impl PdcManager {
    pub const DEFAULT_MAX_LATENCY: usize = 48000 * 10;

    pub fn new(channel_count: usize, return_count: usize) -> Self {
        Self {
            state: ArcSwap::from_pointee(PdcState::new(channel_count, return_count)),
            enabled: AtomicBool::new(true),
            max_allowed_latency: AtomicUsize::new(Self::DEFAULT_MAX_LATENCY),
        }
    }

    #[inline]
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    #[inline]
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    #[inline]
    pub fn max_latency(&self) -> usize {
        self.state.load().max_latency
    }

    pub fn set_max_allowed_latency(&self, samples: usize) {
        self.max_allowed_latency.store(samples, Ordering::Release);
    }

    #[inline]
    pub fn get_snapshot(&self) -> Arc<PdcState> {
        self.state.load_full()
    }

    pub fn set_channel_latency(&self, channel_index: usize, latency_samples: usize) -> usize {
        // Validate latency bounds
        let max_allowed = self.max_allowed_latency.load(Ordering::Relaxed);
        if latency_samples > max_allowed {
            panic!(
                "Track {} reported excessive latency: {} samples ({:.2} seconds @ 48kHz). \
                 Maximum allowed: {} samples ({:.2} seconds). \
                 This indicates a buggy plugin or driver issue.",
                channel_index,
                latency_samples,
                latency_samples as f64 / 48000.0,
                max_allowed,
                max_allowed as f64 / 48000.0
            );
        }

        // Clone current state
        let mut new_state = self.state.load().as_ref().clone();

        // Resize if needed
        if channel_index >= new_state.channel_latencies.len() {
            new_state.channel_latencies.resize(channel_index + 1, 0);
            new_state.channel_compensations.resize(channel_index + 1, 0);
        }

        new_state.channel_latencies[channel_index] = latency_samples;

        // Recalculate compensations
        new_state.recalculate();

        // Swap state (lock-free for readers)
        self.state.store(Arc::new(new_state.clone()));
        new_state.channel_compensations[channel_index]
    }

    pub fn get_channel_compensation(&self, channel_index: usize) -> usize {
        let state = self.state.load();
        state
            .channel_compensations
            .get(channel_index)
            .copied()
            .unwrap_or(0)
    }

    pub fn remove_channel(&self, channel_index: usize) {
        let mut new_state = self.state.load().as_ref().clone();

        if channel_index < new_state.channel_latencies.len() {
            new_state.channel_latencies[channel_index] = 0;
            new_state.recalculate();
            self.state.store(Arc::new(new_state));
        }
    }

    pub fn set_return_latency(&self, return_index: usize, latency_samples: usize) -> usize {
        // Validate latency bounds
        let max_allowed = self.max_allowed_latency.load(Ordering::Relaxed);
        if latency_samples > max_allowed {
            panic!(
                "Return bus {} reported excessive latency: {} samples ({:.2} seconds @ 48kHz). \
                 Maximum allowed: {} samples ({:.2} seconds). \
                 This indicates a buggy effect or driver issue.",
                return_index,
                latency_samples,
                latency_samples as f64 / 48000.0,
                max_allowed,
                max_allowed as f64 / 48000.0
            );
        }

        // Clone current state
        let mut new_state = self.state.load().as_ref().clone();

        // Resize if needed
        if return_index >= new_state.return_latencies.len() {
            new_state.return_latencies.resize(return_index + 1, 0);
            new_state.return_compensations.resize(return_index + 1, 0);
        }

        new_state.return_latencies[return_index] = latency_samples;

        // Recalculate compensations
        new_state.recalculate();

        // Swap state (lock-free for readers)
        self.state.store(Arc::new(new_state.clone()));
        new_state.return_compensations[return_index]
    }

    pub fn get_return_compensation(&self, return_index: usize) -> usize {
        let state = self.state.load();
        state
            .return_compensations
            .get(return_index)
            .copied()
            .unwrap_or(0)
    }

    pub fn remove_return(&self, return_index: usize) {
        let mut new_state = self.state.load().as_ref().clone();

        if return_index < new_state.return_latencies.len() {
            new_state.return_latencies[return_index] = 0;
            new_state.recalculate();
            self.state.store(Arc::new(new_state));
        }
    }

    pub fn resize(&self, channel_count: usize, return_count: usize) {
        let mut new_state = self.state.load().as_ref().clone();

        new_state.channel_latencies.resize(channel_count, 0);
        new_state.channel_compensations.resize(channel_count, 0);
        new_state.return_latencies.resize(return_count, 0);
        new_state.return_compensations.resize(return_count, 0);

        self.state.store(Arc::new(new_state));
    }

    pub fn clear(&self) {
        let state = self.state.load();
        let new_state = PdcState::new(state.channel_latencies.len(), state.return_latencies.len());
        self.state.store(Arc::new(new_state));
    }
}

impl Default for PdcManager {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdc_creation() {
        let pdc = PdcManager::new(4, 2);
        assert_eq!(pdc.max_latency(), 0);
        assert!(pdc.is_enabled());
    }

    #[test]
    fn test_set_channel_latency() {
        let pdc = PdcManager::new(4, 2);

        pdc.set_channel_latency(0, 100);
        pdc.set_channel_latency(1, 200);
        pdc.set_channel_latency(2, 50);

        // Max should be 200
        assert_eq!(pdc.max_latency(), 200);

        // Compensations: max - latency
        assert_eq!(pdc.get_channel_compensation(0), 100); // 200 - 100
        assert_eq!(pdc.get_channel_compensation(1), 0); // 200 - 200
        assert_eq!(pdc.get_channel_compensation(2), 150); // 200 - 50
    }

    #[test]
    fn test_set_return_latency() {
        let pdc = PdcManager::new(2, 4);

        pdc.set_return_latency(0, 150);
        pdc.set_return_latency(1, 100);

        assert_eq!(pdc.max_latency(), 150);
        assert_eq!(pdc.get_return_compensation(0), 0); // 150 - 150
        assert_eq!(pdc.get_return_compensation(1), 50); // 150 - 100
    }

    #[test]
    fn test_mixed_track_return_latency() {
        let pdc = PdcManager::new(2, 2);

        pdc.set_channel_latency(0, 100);
        pdc.set_return_latency(0, 200);

        // Max should be 200 (from return bus)
        assert_eq!(pdc.max_latency(), 200);

        // Track compensation: 200 - 100 = 100
        assert_eq!(pdc.get_channel_compensation(0), 100);

        assert_eq!(pdc.get_return_compensation(0), 0);
    }

    #[test]
    fn test_enable_disable() {
        let pdc = PdcManager::new(2, 2);

        assert!(pdc.is_enabled());

        pdc.set_enabled(false);
        assert!(!pdc.is_enabled());

        pdc.set_enabled(true);
        assert!(pdc.is_enabled());
    }

    #[test]
    fn test_remove_channel() {
        let pdc = PdcManager::new(4, 2);

        pdc.set_channel_latency(0, 100);
        pdc.set_channel_latency(1, 200);

        assert_eq!(pdc.max_latency(), 200);

        pdc.remove_channel(1);

        // Max should now be 100
        assert_eq!(pdc.max_latency(), 100);
        assert_eq!(pdc.get_channel_compensation(0), 0); // 100 - 100
    }

    #[test]
    fn test_resize() {
        let pdc = PdcManager::new(2, 2);

        pdc.set_channel_latency(0, 100);

        // Resize to accommodate more tracks
        pdc.resize(10, 5);

        // Old data should still be there
        let state = pdc.get_snapshot();
        assert_eq!(state.channel_latencies.len(), 10);
        assert_eq!(state.return_latencies.len(), 5);
        assert_eq!(state.channel_latencies[0], 100);
    }

    #[test]
    fn test_clear() {
        let pdc = PdcManager::new(4, 2);

        pdc.set_channel_latency(0, 100);
        pdc.set_return_latency(0, 200);

        assert_eq!(pdc.max_latency(), 200);

        pdc.clear();

        assert_eq!(pdc.max_latency(), 0);
        assert_eq!(pdc.get_channel_compensation(0), 0);
        assert_eq!(pdc.get_return_compensation(0), 0);
    }

    #[test]
    #[should_panic(expected = "excessive latency")]
    fn test_excessive_track_latency() {
        let pdc = PdcManager::new(2, 2);

        // Try to set latency above default maximum (10 seconds @ 48kHz)
        pdc.set_channel_latency(0, PdcManager::DEFAULT_MAX_LATENCY + 1);
    }

    #[test]
    #[should_panic(expected = "excessive latency")]
    fn test_excessive_return_latency() {
        let pdc = PdcManager::new(2, 2);

        // Try to set latency above default maximum
        pdc.set_return_latency(0, PdcManager::DEFAULT_MAX_LATENCY + 1);
    }

    #[test]
    fn test_lock_free_snapshot() {
        let pdc = PdcManager::new(4, 2);

        pdc.set_channel_latency(0, 100);
        pdc.set_channel_latency(1, 200);

        let snapshot = pdc.get_snapshot();

        assert_eq!(snapshot.max_latency, 200);
        assert_eq!(snapshot.channel_latencies[0], 100);
        assert_eq!(snapshot.channel_latencies[1], 200);
        assert_eq!(snapshot.channel_compensations[0], 100);
        assert_eq!(snapshot.channel_compensations[1], 0);
    }
}
