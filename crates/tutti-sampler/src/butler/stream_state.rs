//! Channel streaming state for disk-based audio playback.

use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;

use super::prefetch::RegionBufferConsumer;

/// Channel streaming state using ring buffers for disk streaming.
pub struct ChannelStreamState {
    _channel_index: usize,
    current_file: Option<PathBuf>,
    ring_buffer_consumer: Option<Arc<Mutex<RegionBufferConsumer>>>,
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
        self.current_file = Some(file_path);
        self.ring_buffer_consumer = Some(consumer);
        self.speed = speed;
        self.gain = gain;
    }

    pub fn stop_streaming(&mut self) {
        self.current_file = None;
        self.ring_buffer_consumer = None;
    }

    pub fn flush_buffer(&self) {
        if let Some(ref consumer) = self.ring_buffer_consumer {
            let mut consumer_guard = consumer.lock();
            while consumer_guard.read().is_some() {}
        }
    }

    pub fn region_id(&self) -> Option<super::request::RegionId> {
        self.ring_buffer_consumer.as_ref().map(|c| {
            let guard = c.lock();
            guard.region_id()
        })
    }

    pub fn check_loop_condition(&self) -> Option<u64> {
        if let Some((loop_start, loop_end)) = self.loop_range {
            if let Some(ref consumer) = self.ring_buffer_consumer {
                let guard = consumer.lock();
                let read_pos = guard.read_position();

                if read_pos >= loop_end {
                    return Some(loop_start);
                }
            }
        }
        None
    }
}

