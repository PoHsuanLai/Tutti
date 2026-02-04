//! Loop handling and crossfade capture for butler thread.

use super::cache::LruCache;
use super::metrics::IOMetrics;
use super::prefetch::RegionBufferProducer;
use super::refill::get_wave_from_cache;
use super::request::RegionId;
use super::stream_state::{ChannelStreamState, LoopStatus};
use dashmap::DashMap;
use std::path::PathBuf;
use tutti_core::Wave;

/// Check and handle stream loop conditions with crossfade support.
///
/// Loop crossfade is now handled via SharedStreamState for lock-free audio thread access.
/// Butler captures fadeout/fadein samples and passes them to SharedStreamState.
pub(super) fn check_and_handle_loops(
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
) {
    for stream_entry in stream_states.iter() {
        let stream_state = stream_entry.value();
        let loop_status = stream_state.check_loop_status();

        match loop_status {
            LoopStatus::Normal => continue,
            LoopStatus::ApproachingEnd => {
                if stream_state.shared_state().is_loop_crossfading() {
                    continue;
                }

                let fade_len = stream_state.loop_crossfade_samples();
                if fade_len == 0 {
                    continue;
                }

                let Some((loop_start, loop_end)) = stream_state.loop_range() else {
                    continue;
                };
                let Some(region_id) = stream_state.region_id() else {
                    continue;
                };

                if let Some(&idx) = producer_index.get(&region_id) {
                    let file_path = producers[idx].file_path();
                    if let Some(wave) = get_wave_from_cache(sample_cache, metrics, file_path) {
                        let fadeout_start = (loop_end as usize).saturating_sub(fade_len);
                        let fadeout = capture_samples(&wave, fadeout_start, fade_len);

                        let fadein = if let Some(preloop) = stream_state.preloop_buffer() {
                            preloop.to_vec()
                        } else {
                            capture_samples(&wave, loop_start as usize, fade_len)
                        };

                        stream_state
                            .shared_state()
                            .start_loop_crossfade(fadeout, fadein);
                    }
                }
            }
            LoopStatus::AtEnd(loop_start) => {
                stream_state.shared_state().clear_loop_crossfade();

                if let Some(region_id) = stream_state.region_id() {
                    if let Some(&idx) = producer_index.get(&region_id) {
                        stream_state.flush_buffer();
                        producers[idx].set_file_position(loop_start);
                    }
                }
            }
        }
    }
}

/// Capture samples from a wave file into a Vec for crossfade.
pub(super) fn capture_samples(wave: &Wave, start: usize, count: usize) -> Vec<(f32, f32)> {
    let mut samples = Vec::with_capacity(count);
    let channels = wave.channels();
    for i in 0..count {
        let idx = start + i;
        if idx < wave.len() {
            let left = wave.at(0, idx);
            let right = if channels > 1 { wave.at(1, idx) } else { left };
            samples.push((left, right));
        } else {
            samples.push((0.0, 0.0));
        }
    }
    samples
}

/// Capture samples from the current ring buffer for fadeout during seek.
///
/// Reads the last N samples that would have been played from the ring buffer.
pub(super) fn capture_fadeout_samples(
    stream_state: &ChannelStreamState,
    count: usize,
) -> Vec<(f32, f32)> {
    if count == 0 {
        return Vec::new();
    }

    let Some(consumer_arc) = stream_state.consumer() else {
        return Vec::new();
    };

    let Some(mut consumer) = consumer_arc.try_lock() else {
        return Vec::new();
    };

    let available = consumer.available();
    let to_read = available.min(count);

    if to_read == 0 {
        return Vec::new();
    }

    let mut samples = Vec::with_capacity(to_read);
    for _ in 0..to_read {
        if let Some(sample) = consumer.read() {
            samples.push(sample);
        } else {
            break;
        }
    }

    if samples.len() < count {
        let pad_sample = samples.last().copied().unwrap_or((0.0, 0.0));
        samples.resize(count, pad_sample);
    }

    samples
}

/// Capture samples from the Wave file at the new seek position for fadein.
pub(super) fn capture_fadein_samples(
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    file_path: &PathBuf,
    position_samples: u64,
    count: usize,
) -> Vec<(f32, f32)> {
    if count == 0 {
        return Vec::new();
    }

    let Some(wave) = get_wave_from_cache(sample_cache, metrics, file_path) else {
        return Vec::new();
    };

    let mut samples = Vec::with_capacity(count);
    let channels = wave.channels();

    for i in 0..count {
        let idx = position_samples as usize + i;
        if idx >= wave.len() {
            samples.push((0.0, 0.0));
        } else {
            let left = wave.at(0, idx);
            let right = if channels > 1 { wave.at(1, idx) } else { left };
            samples.push((left, right));
        }
    }

    samples
}

/// Calculate optimal buffer size based on file size.
pub(super) fn calculate_buffer_size(file_length_samples: u64, sample_rate: f64) -> usize {
    let file_size_bytes = file_length_samples * 2 * 4;
    let file_size_mb = file_size_bytes as f64 / (1024.0 * 1024.0);

    let buffer_seconds = if file_size_mb < 50.0 {
        (file_length_samples as f64 / sample_rate).min(30.0)
    } else if file_size_mb < 200.0 {
        10.0
    } else if file_size_mb < 500.0 {
        5.0
    } else {
        3.0
    };

    let buffer_capacity = (buffer_seconds * sample_rate) as usize;
    buffer_capacity.max(4096)
}
