//! Ring buffer refill logic for butler thread.

use super::cache::LruCache;
use super::metrics::IOMetrics;
use super::prefetch::RegionBufferProducer;
use super::request::RegionId;
use super::stream_state::ChannelStreamState;
use dashmap::DashMap;
use rayon::prelude::*;
use std::path::PathBuf;
use std::sync::Arc;
use tutti_core::Wave;

/// Calculate optimal chunk size using varifill strategy.
///
/// Adapts chunk size based on:
/// - Buffer urgency (how empty the buffer is)
/// - Disk bandwidth (recent read throughput)
/// - Playback speed (varispeed)
///
/// Returns chunk size in samples.
#[inline]
pub(super) fn calculate_varifill_chunk(
    buffer_fill: f32,
    base_chunk: usize,
    read_rate_bytes_per_sec: f64,
    playback_speed: f32,
) -> usize {
    let urgency = (1.0 - buffer_fill) as f64;

    const BASELINE_RATE: f64 = 10_000_000.0;
    let bandwidth_factor = if read_rate_bytes_per_sec > 0.0 {
        (read_rate_bytes_per_sec / BASELINE_RATE)
            .sqrt()
            .clamp(0.5, 2.0)
    } else {
        1.0
    };

    let speed_factor = playback_speed.max(1.0) as f64;

    let multiplier = (0.5 + urgency * 1.5) * bandwidth_factor * speed_factor;

    let clamped = multiplier.clamp(0.25, 4.0);

    let chunk_size = (base_chunk as f64 * clamped) as usize;

    chunk_size.max(1024)
}

/// Refill ring buffers from disk with varifill strategy.
///
/// Uses a pre-allocated buffer to avoid allocation in the hot path.
/// Loop crossfade is handled by the audio thread via SharedStreamState.
///
/// Chunk size is dynamically adjusted (varifill) based on:
/// - Buffer urgency (how empty the buffer is)
/// - Disk throughput (recent read rate)
/// - Playback speed (varispeed)
#[allow(clippy::too_many_arguments)]
pub(super) fn refill_all_streams(
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    base_chunk_size: usize,
    buffer_margin: f64,
    interleave_buffer: &mut Vec<(f32, f32)>,
) {
    let read_rate = metrics.read_rate();

    for entry in stream_states.iter() {
        let stream_state = entry.value();

        let region_id = match stream_state.region_id() {
            Some(id) => id,
            None => continue,
        };

        let idx = match producer_index.get(&region_id) {
            Some(&i) => i,
            None => continue,
        };
        let producer = &mut producers[idx];

        let available = producer.capacity() - producer.write_space();
        let buffer_capacity = producer.capacity();

        let fill_pct = available as f32 / buffer_capacity as f32;

        stream_state.shared_state().set_buffer_fill(fill_pct);

        // margin > 1.0 keeps more data buffered to handle external sync jitter
        let fill_threshold = (0.75 / buffer_margin) as f32;

        if fill_pct >= fill_threshold {
            continue;
        }

        if fill_pct < 0.10 {
            metrics.record_low_buffer();
        }

        let is_reverse = stream_state.is_reverse();
        let speed = stream_state.speed();
        let src_ratio = stream_state.shared_state().src_ratio();

        // SRC ratio > 1.0 means audio thread consumes ring buffer faster
        let adjusted_speed = speed * src_ratio * buffer_margin as f32;
        let chunk_size =
            calculate_varifill_chunk(fill_pct, base_chunk_size, read_rate, adjusted_speed);

        let file_path = producer.file_path();
        let wave = match get_wave_from_cache(sample_cache, metrics, file_path) {
            Some(w) => w,
            None => continue,
        };

        let file_position = producer.file_position() as usize;
        let channels = wave.channels();

        if is_reverse {
            refill_reverse(
                producer,
                &wave,
                file_position,
                chunk_size,
                channels,
                interleave_buffer,
            );
        } else {
            refill_forward(
                producer,
                &wave,
                file_position,
                chunk_size,
                channels,
                interleave_buffer,
            );
        }
    }
}

/// Work item for parallel refill - contains everything needed to refill one stream.
struct RefillWorkItem {
    /// Index into the producers Vec
    producer_idx: usize,
    /// Number of samples to read
    chunk_size: usize,
    /// Whether playing in reverse
    is_reverse: bool,
    /// Path to audio file
    file_path: PathBuf,
    /// Current buffer fill percentage (0.0-1.0)
    fill_pct: f32,
    /// Shared state for reporting buffer fill
    shared: Arc<super::shared_state::SharedStreamState>,
}

/// Parallel refill using rayon's par_iter_mut with varifill strategy.
///
/// Uses Vec<RegionBufferProducer> with par_iter_mut which only requires Send, not Sync.
/// Each rayon worker gets exclusive &mut access to a different producer.
/// Only used when parallel_io is enabled and there are 3+ streams.
pub(super) fn refill_all_streams_parallel(
    stream_states: &DashMap<usize, ChannelStreamState>,
    producers: &mut [RegionBufferProducer],
    producer_index: &std::collections::HashMap<RegionId, usize>,
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    base_chunk_size: usize,
    buffer_margin: f64,
) {
    let read_rate = metrics.read_rate();

    let fill_threshold = (0.75 / buffer_margin) as f32;

    let work_items: Vec<RefillWorkItem> = stream_states
        .iter()
        .filter_map(|entry| {
            let stream_state = entry.value();
            let region_id = stream_state.region_id()?;
            let &idx = producer_index.get(&region_id)?;
            let producer = &producers[idx];

            let available = producer.capacity() - producer.write_space();
            let fill_pct = available as f32 / producer.capacity() as f32;

            if fill_pct >= fill_threshold {
                return None;
            }

            let speed = stream_state.speed();
            let src_ratio = stream_state.shared_state().src_ratio();

            let adjusted_speed = speed * src_ratio * buffer_margin as f32;
            let chunk_size =
                calculate_varifill_chunk(fill_pct, base_chunk_size, read_rate, adjusted_speed);

            let shared = stream_state.shared_state();

            Some(RefillWorkItem {
                producer_idx: idx,
                chunk_size,
                is_reverse: stream_state.is_reverse(),
                file_path: producer.file_path().to_path_buf(),
                fill_pct,
                shared,
            })
        })
        .collect();

    for item in &work_items {
        if item.fill_pct < 0.10 {
            metrics.record_low_buffer();
        }
    }

    let work_by_idx: std::collections::HashMap<usize, &RefillWorkItem> = work_items
        .iter()
        .map(|item| (item.producer_idx, item))
        .collect();

    producers
        .par_iter_mut()
        .enumerate()
        .for_each(|(idx, producer)| {
            let Some(item) = work_by_idx.get(&idx) else {
                return;
            };

            thread_local! {
                static LOCAL_BUF: std::cell::RefCell<Vec<(f32, f32)>> =
                    std::cell::RefCell::new(Vec::with_capacity(16384));
            }

            LOCAL_BUF.with(|buf| {
                let mut buf = buf.borrow_mut();
                refill_single_stream_with_producer(
                    producer,
                    sample_cache,
                    item.chunk_size,
                    item.is_reverse,
                    &item.file_path,
                    item.fill_pct,
                    &item.shared,
                    &mut buf,
                );
            });
        });
}

/// Refill a single stream with the provided producer and buffer (used by parallel path).
///
/// Takes a direct mutable reference to the producer (from par_iter_mut).
#[allow(clippy::too_many_arguments)]
fn refill_single_stream_with_producer(
    producer: &mut RegionBufferProducer,
    sample_cache: &LruCache,
    chunk_size: usize,
    is_reverse: bool,
    file_path: &PathBuf,
    fill_pct: f32,
    shared: &super::shared_state::SharedStreamState,
    buffer: &mut Vec<(f32, f32)>,
) {
    let wave = match sample_cache.get(file_path) {
        Some(w) => w,
        None => return,
    };

    shared.set_buffer_fill(fill_pct);

    let file_position = producer.file_position() as usize;
    let channels = wave.channels();

    buffer.clear();

    if is_reverse {
        fill_buffer_reverse(&wave, file_position, chunk_size, channels, buffer);
        let written = producer.write(buffer);
        producer.set_file_position(file_position.saturating_sub(written) as u64);
    } else {
        fill_buffer_forward(&wave, file_position, chunk_size, channels, buffer);
        let written = producer.write(buffer);
        producer.set_file_position((file_position + written) as u64);
    }
}

/// Fill buffer with forward samples (no ring buffer write).
#[inline]
fn fill_buffer_forward(
    wave: &Wave,
    file_position: usize,
    chunk_size: usize,
    channels: usize,
    buffer: &mut Vec<(f32, f32)>,
) {
    for i in 0..chunk_size {
        let sample_idx = file_position + i;
        let sample = if sample_idx >= wave.len() {
            (0.0, 0.0)
        } else {
            let left = wave.at(0, sample_idx);
            let right = if channels > 1 {
                wave.at(1, sample_idx)
            } else {
                left
            };
            (left, right)
        };
        buffer.push(sample);
    }
}

/// Fill buffer with reversed samples (no ring buffer write).
#[inline]
fn fill_buffer_reverse(
    wave: &Wave,
    file_position: usize,
    chunk_size: usize,
    channels: usize,
    buffer: &mut Vec<(f32, f32)>,
) {
    let read_start = file_position.saturating_sub(chunk_size);
    let actual_chunk = file_position - read_start;

    if actual_chunk == 0 {
        for _ in 0..chunk_size {
            buffer.push((0.0, 0.0));
        }
        return;
    }

    let mut temp = Vec::with_capacity(actual_chunk);
    for i in 0..actual_chunk {
        let sample_idx = read_start + i;
        let left = wave.at(0, sample_idx);
        let right = if channels > 1 {
            wave.at(1, sample_idx)
        } else {
            left
        };
        temp.push((left, right));
    }

    for sample in temp.into_iter().rev() {
        buffer.push(sample);
    }
}

/// Refill buffer for forward playback.
///
/// Crossfade is now handled by the audio thread via SharedStreamState for lock-free access.
pub(super) fn refill_forward(
    producer: &mut RegionBufferProducer,
    wave: &Wave,
    file_position: usize,
    chunk_size: usize,
    channels: usize,
    interleave_buffer: &mut Vec<(f32, f32)>,
) {
    interleave_buffer.clear();

    for i in 0..chunk_size {
        let sample_idx = file_position + i;

        let sample = if sample_idx >= wave.len() {
            (0.0, 0.0)
        } else {
            let left = wave.at(0, sample_idx);
            let right = if channels > 1 {
                wave.at(1, sample_idx)
            } else {
                left
            };
            (left, right)
        };

        interleave_buffer.push(sample);
    }

    let written = producer.write(interleave_buffer);
    producer.set_file_position((file_position + written) as u64);
}

/// Refill buffer for reverse playback.
/// Reads samples forward from disk, then writes them reversed to the ring buffer.
pub(super) fn refill_reverse(
    producer: &mut RegionBufferProducer,
    wave: &Wave,
    file_position: usize,
    chunk_size: usize,
    channels: usize,
    interleave_buffer: &mut Vec<(f32, f32)>,
) {
    let read_start = file_position.saturating_sub(chunk_size);
    let actual_chunk = file_position - read_start;

    if actual_chunk == 0 {
        interleave_buffer.clear();
        for _ in 0..chunk_size {
            interleave_buffer.push((0.0, 0.0));
        }
        producer.write(interleave_buffer);
        return;
    }

    interleave_buffer.clear();
    for i in 0..actual_chunk {
        let sample_idx = read_start + i;
        let left = wave.at(0, sample_idx);
        let right = if channels > 1 {
            wave.at(1, sample_idx)
        } else {
            left
        };
        interleave_buffer.push((left, right));
    }

    let written = producer.write_reversed(interleave_buffer);
    producer.set_file_position(file_position.saturating_sub(written) as u64);
}

/// Helper: Get wave from cache or load from disk.
pub(super) fn get_wave_from_cache(
    sample_cache: &LruCache,
    metrics: &IOMetrics,
    file_path: &PathBuf,
) -> Option<Arc<Wave>> {
    if let Some(cached) = sample_cache.get(file_path) {
        metrics.record_cache_hit();
        Some(cached)
    } else {
        metrics.record_cache_miss();
        match Wave::load(file_path) {
            Ok(w) => {
                let arc_wave = Arc::new(w);
                let bytes = arc_wave.len() as u64 * arc_wave.channels() as u64 * 4;
                metrics.record_read(bytes);
                sample_cache.insert(file_path.clone(), arc_wave.clone());
                Some(arc_wave)
            }
            Err(_) => None,
        }
    }
}
