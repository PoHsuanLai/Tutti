//! Channel streaming state for disk-based audio playback.

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tutti_core::{AtomicU64, Ordering};

use super::prefetch::RegionBufferConsumer;
use super::request::RegionId;

/// Channel streaming state using ring buffers for disk streaming.
pub struct ChannelStreamState {
    _channel_index: usize,
    current_file: Option<PathBuf>,
    ring_buffer_consumer: Option<Arc<Mutex<RegionBufferConsumer>>>,
    /// Cached for lock-free access from butler thread.
    cached_region_id: Option<RegionId>,
    /// Shared read position — lock-free access without acquiring the consumer Mutex.
    cached_read_position: Option<Arc<AtomicU64>>,
    speed: f32,
    gain: f32,
    loop_range: Option<(u64, u64)>,
    _sample_rate: f64,
}

impl ChannelStreamState {
    pub fn new(channel_index: usize, sample_rate: f64) -> Self {
        Self {
            _channel_index: channel_index,
            current_file: None,
            ring_buffer_consumer: None,
            cached_region_id: None,
            cached_read_position: None,
            speed: 1.0,
            gain: 1.0,
            loop_range: None,
            _sample_rate: sample_rate,
        }
    }

    pub fn start_streaming(
        &mut self,
        file_path: PathBuf,
        consumer: Arc<Mutex<RegionBufferConsumer>>,
        speed: f32,
        gain: f32,
    ) {
        // Cache region_id and read_position for lock-free access
        {
            let guard = consumer.lock();
            self.cached_region_id = Some(guard.region_id());
            self.cached_read_position = Some(guard.read_position_shared());
        }
        self.current_file = Some(file_path);
        self.ring_buffer_consumer = Some(consumer);
        self.speed = speed;
        self.gain = gain;
    }

    pub fn stop_streaming(&mut self) {
        self.current_file = None;
        self.ring_buffer_consumer = None;
        self.cached_region_id = None;
        self.cached_read_position = None;
    }

    /// Clear all buffered samples. Uses `RegionBufferConsumer::clear()` instead of
    /// busy-waiting on `read()` — bounded and much shorter lock hold time.
    pub fn flush_buffer(&self) {
        if let Some(ref consumer) = self.ring_buffer_consumer {
            let mut consumer_guard = consumer.lock();
            consumer_guard.clear();
        }
    }

    /// Lock-free: returns cached region_id without acquiring the consumer Mutex.
    pub fn region_id(&self) -> Option<RegionId> {
        self.cached_region_id
    }

    /// Lock-free: reads the shared AtomicU64 read_position without acquiring the consumer Mutex.
    pub fn check_loop_condition(&self) -> Option<u64> {
        if let Some((loop_start, loop_end)) = self.loop_range {
            if let Some(ref pos) = self.cached_read_position {
                let read_pos = pos.load(Ordering::Relaxed);
                if read_pos >= loop_end {
                    return Some(loop_start);
                }
            }
        }
        None
    }
}
