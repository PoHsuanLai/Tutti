//! Channel streaming state for disk-based audio playback.

use parking_lot::Mutex;
use std::sync::Arc;
use tutti_core::{AtomicU64, Ordering};

use super::prefetch::RegionBufferConsumer;
use super::request::RegionId;
use super::shared_state::SharedStreamState;
use super::varispeed::Varispeed;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LoopStatus {
    Normal,
    /// Within crossfade distance of loop end
    ApproachingEnd,
    /// At or past loop end; value is loop start position to wrap to
    AtEnd(u64),
}

pub struct ChannelStreamState {
    ring_buffer_consumer: Option<Arc<Mutex<RegionBufferConsumer>>>,
    /// Lock-free access from butler thread without acquiring consumer Mutex.
    cached_region_id: Option<RegionId>,
    cached_read_position: Option<Arc<AtomicU64>>,
    loop_range: Option<(u64, u64)>,
    loop_crossfade_samples: usize,
    /// Cached fadein samples from loop start, avoids re-reading on each loop.
    preloop_buffer: Option<Vec<(f32, f32)>>,
    varispeed: Varispeed,
    shared_state: Arc<SharedStreamState>,
    /// How much earlier to read for delay compensation.
    pdc_preroll: u64,
}

impl Default for ChannelStreamState {
    fn default() -> Self {
        Self {
            ring_buffer_consumer: None,
            cached_region_id: None,
            cached_read_position: None,
            loop_range: None,
            loop_crossfade_samples: 0,
            preloop_buffer: None,
            varispeed: Varispeed::default(),
            shared_state: Arc::new(SharedStreamState::new()),
            pdc_preroll: 0,
        }
    }
}

impl ChannelStreamState {
    pub fn start_streaming(&mut self, consumer: Arc<Mutex<RegionBufferConsumer>>) {
        {
            let guard = consumer.lock();
            self.cached_region_id = Some(guard.region_id());
            self.cached_read_position = Some(guard.read_position_shared());
        }
        self.ring_buffer_consumer = Some(consumer);
    }

    pub fn stop_streaming(&mut self) {
        self.ring_buffer_consumer = None;
        self.cached_region_id = None;
        self.cached_read_position = None;
        self.loop_range = None;
        self.loop_crossfade_samples = 0;
        self.preloop_buffer = None;
        self.varispeed = Varispeed::default();
        self.pdc_preroll = 0;
        self.shared_state.set_speed(1.0);
        self.shared_state.set_reverse(false);
        self.shared_state.set_seeking(false);
        self.shared_state.set_src_ratio(1.0);
        self.shared_state.clear_loop_crossfade();
    }

    pub fn shared_state(&self) -> Arc<SharedStreamState> {
        Arc::clone(&self.shared_state)
    }

    pub fn consumer(&self) -> Option<Arc<Mutex<RegionBufferConsumer>>> {
        self.ring_buffer_consumer.clone()
    }

    pub fn is_streaming(&self) -> bool {
        self.ring_buffer_consumer.is_some()
    }

    /// Uses `clear()` instead of busy-waiting on `read()` for bounded lock hold time.
    pub fn flush_buffer(&self) {
        if let Some(ref consumer) = self.ring_buffer_consumer {
            let mut consumer_guard = consumer.lock();
            consumer_guard.clear();
        }
    }

    pub(crate) fn region_id(&self) -> Option<RegionId> {
        self.cached_region_id
    }

    pub fn set_loop_range(&mut self, start: u64, end: u64, crossfade_samples: usize) {
        self.loop_range = Some((start, end));
        self.loop_crossfade_samples = crossfade_samples;
    }

    pub fn clear_loop_range(&mut self) {
        self.loop_range = None;
        self.loop_crossfade_samples = 0;
        self.preloop_buffer = None;
        self.shared_state.clear_loop_crossfade();
    }

    pub fn loop_range(&self) -> Option<(u64, u64)> {
        self.loop_range
    }

    pub fn loop_crossfade_samples(&self) -> usize {
        self.loop_crossfade_samples
    }

    pub fn set_preloop_buffer(&mut self, samples: Vec<(f32, f32)>) {
        self.preloop_buffer = Some(samples);
    }

    pub fn preloop_buffer(&self) -> Option<&[(f32, f32)]> {
        self.preloop_buffer.as_deref()
    }

    pub fn check_loop_status(&self) -> LoopStatus {
        let Some((loop_start, loop_end)) = self.loop_range else {
            return LoopStatus::Normal;
        };

        let Some(ref pos) = self.cached_read_position else {
            return LoopStatus::Normal;
        };

        let read_pos = pos.load(Ordering::Relaxed);

        if read_pos >= loop_end {
            return LoopStatus::AtEnd(loop_start);
        }

        if self.loop_crossfade_samples > 0 {
            let crossfade_start = loop_end.saturating_sub(self.loop_crossfade_samples as u64);
            if read_pos >= crossfade_start {
                return LoopStatus::ApproachingEnd;
            }
        }

        LoopStatus::Normal
    }

    pub fn set_varispeed(&mut self, varispeed: Varispeed) {
        self.varispeed = varispeed;
        self.shared_state.set_speed(varispeed.speed);
        self.shared_state.set_reverse(varispeed.is_reverse());
    }

    pub fn is_reverse(&self) -> bool {
        self.varispeed.is_reverse()
    }

    pub fn speed(&self) -> f32 {
        self.varispeed.effective_speed()
    }

    pub fn set_seeking(&self, seeking: bool) {
        self.shared_state.set_seeking(seeking);
    }

    pub fn set_pdc_preroll(&mut self, samples: u64) {
        self.pdc_preroll = samples;
    }

    pub fn pdc_preroll(&self) -> u64 {
        self.pdc_preroll
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loop_status_normal() {
        let state = ChannelStreamState::default();
        assert_eq!(state.check_loop_status(), LoopStatus::Normal);
    }

    #[test]
    fn test_varispeed_default() {
        let state = ChannelStreamState::default();
        assert!(!state.is_reverse());
        assert_eq!(state.speed(), 1.0);
    }

    #[test]
    fn test_set_varispeed() {
        let mut state = ChannelStreamState::default();
        state.set_varispeed(Varispeed::reverse());
        assert!(state.is_reverse());
    }

    #[test]
    fn test_loop_range() {
        let mut state = ChannelStreamState::default();
        assert!(state.loop_range().is_none());

        state.set_loop_range(1000, 5000, 64);
        assert_eq!(state.loop_range(), Some((1000, 5000)));
        assert_eq!(state.loop_crossfade_samples(), 64);

        state.clear_loop_range();
        assert!(state.loop_range().is_none());
        assert_eq!(state.loop_crossfade_samples(), 0);
    }

    #[test]
    fn test_pdc_preroll_default() {
        let state = ChannelStreamState::default();
        assert_eq!(state.pdc_preroll(), 0);
    }

    #[test]
    fn test_pdc_preroll_set() {
        let mut state = ChannelStreamState::default();
        state.set_pdc_preroll(1000);
        assert_eq!(state.pdc_preroll(), 1000);

        state.set_pdc_preroll(500);
        assert_eq!(state.pdc_preroll(), 500);
    }

    #[test]
    fn test_pdc_preroll_reset_on_stop() {
        let mut state = ChannelStreamState::default();
        state.set_pdc_preroll(1000);
        assert_eq!(state.pdc_preroll(), 1000);

        state.stop_streaming();
        assert_eq!(state.pdc_preroll(), 0);
    }

    #[test]
    fn test_preloop_buffer() {
        let mut state = ChannelStreamState::default();

        assert!(!state.preloop_buffer().is_some());
        assert!(state.preloop_buffer().is_none());

        let samples = vec![(0.5, 0.5), (0.6, 0.6), (0.7, 0.7)];
        state.set_preloop_buffer(samples.clone());

        assert!(state.preloop_buffer().is_some());
        assert_eq!(state.preloop_buffer(), Some(samples.as_slice()));

        state.set_loop_range(100, 500, 64);
        state.clear_loop_range();
        assert!(!state.preloop_buffer().is_some());
    }

    #[test]
    fn test_preloop_buffer_reset_on_stop() {
        let mut state = ChannelStreamState::default();
        state.set_preloop_buffer(vec![(0.5, 0.5)]);
        assert!(state.preloop_buffer().is_some());

        state.stop_streaming();
        assert!(!state.preloop_buffer().is_some());
    }
}
