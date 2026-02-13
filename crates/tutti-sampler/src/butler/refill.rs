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

        // Get loop range to respect loop boundaries during refill
        let loop_range = stream_state.loop_range();

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
                loop_range,
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

/// Refill buffer for forward playback, respecting loop boundaries if set.
#[allow(clippy::too_many_arguments)]
pub(super) fn refill_forward(
    producer: &mut RegionBufferProducer,
    wave: &Wave,
    file_position: usize,
    chunk_size: usize,
    channels: usize,
    interleave_buffer: &mut Vec<(f32, f32)>,
    loop_range: Option<(u64, u64)>,
) {
    interleave_buffer.clear();

    // Extract loop bounds if set and valid
    let loop_bounds = loop_range.and_then(|(start, end)| {
        let start = start as usize;
        let end = end as usize;
        let len = end.saturating_sub(start);
        (len > 0).then_some((start, end, len))
    });

    let mut pos = file_position;

    for _ in 0..chunk_size {
        // Wrap position if looping
        if let Some((loop_start, loop_end, loop_len)) = loop_bounds {
            if pos >= loop_end {
                pos = loop_start + ((pos - loop_start) % loop_len);
            }
        }

        let sample = if pos >= wave.len() {
            (0.0, 0.0)
        } else {
            let left = wave.at(0, pos);
            let right = if channels > 1 { wave.at(1, pos) } else { left };
            (left, right)
        };

        interleave_buffer.push(sample);
        pos += 1;
    }

    let written = producer.write(interleave_buffer);

    // Calculate new file position, wrapping if looping
    let mut new_pos = file_position + written;
    if let Some((loop_start, loop_end, loop_len)) = loop_bounds {
        if new_pos >= loop_end {
            new_pos = loop_start + ((new_pos - loop_start) % loop_len);
        }
    }
    producer.set_file_position(new_pos as u64);
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // calculate_varifill_chunk tests
    // =========================================================================

    #[test]
    fn test_varifill_empty_buffer_increases_chunk() {
        // Empty buffer (fill=0) should have high urgency -> larger chunk
        let base = 4096;
        let chunk = calculate_varifill_chunk(0.0, base, 10_000_000.0, 1.0);

        // urgency=1.0, multiplier = (0.5 + 1.0*1.5) * 1.0 * 1.0 = 2.0
        assert!(chunk > base, "Empty buffer should increase chunk size");
        assert_eq!(chunk, base * 2);
    }

    #[test]
    fn test_varifill_full_buffer_decreases_chunk() {
        // Full buffer (fill=1) should have low urgency -> smaller chunk
        let base = 4096;
        let chunk = calculate_varifill_chunk(1.0, base, 10_000_000.0, 1.0);

        // urgency=0.0, multiplier = (0.5 + 0.0*1.5) * 1.0 * 1.0 = 0.5
        assert!(chunk < base, "Full buffer should decrease chunk size");
        assert_eq!(chunk, base / 2);
    }

    #[test]
    fn test_varifill_half_buffer_near_base() {
        // Half-full buffer should be close to base chunk
        let base = 4096;
        let chunk = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);

        // urgency=0.5, multiplier = (0.5 + 0.5*1.5) * 1.0 * 1.0 = 1.25
        assert_eq!(chunk, (base as f64 * 1.25) as usize);
    }

    #[test]
    fn test_varifill_high_speed_increases_chunk() {
        // High playback speed should increase chunk to keep up
        let base = 4096;
        let normal = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);
        let fast = calculate_varifill_chunk(0.5, base, 10_000_000.0, 2.0);

        assert!(fast > normal, "Higher speed should increase chunk");
        assert_eq!(fast, normal * 2);
    }

    #[test]
    fn test_varifill_slow_speed_no_decrease() {
        // Slow speed (<1.0) should NOT decrease chunk (use max(1.0))
        let base = 4096;
        let normal = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);
        let slow = calculate_varifill_chunk(0.5, base, 10_000_000.0, 0.5);

        assert_eq!(
            slow, normal,
            "Slow speed should not decrease chunk below normal"
        );
    }

    #[test]
    fn test_varifill_high_bandwidth_increases_chunk() {
        // High disk throughput allows larger chunks
        let base = 4096;
        let normal = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);
        let fast_disk = calculate_varifill_chunk(0.5, base, 40_000_000.0, 1.0); // 4x bandwidth

        // bandwidth_factor = sqrt(4) = 2.0
        assert!(
            fast_disk > normal,
            "Higher bandwidth should allow larger chunks"
        );
    }

    #[test]
    fn test_varifill_low_bandwidth_decreases_chunk() {
        // Low disk throughput should use smaller chunks
        let base = 4096;
        let normal = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);
        let slow_disk = calculate_varifill_chunk(0.5, base, 2_500_000.0, 1.0); // 0.25x bandwidth

        // bandwidth_factor = sqrt(0.25) = 0.5
        assert!(
            slow_disk < normal,
            "Lower bandwidth should use smaller chunks"
        );
    }

    #[test]
    fn test_varifill_zero_bandwidth_uses_default() {
        // Zero bandwidth should use factor of 1.0
        let base = 4096;
        let normal = calculate_varifill_chunk(0.5, base, 10_000_000.0, 1.0);
        let zero_bw = calculate_varifill_chunk(0.5, base, 0.0, 1.0);

        assert_eq!(zero_bw, normal, "Zero bandwidth should use default factor");
    }

    #[test]
    fn test_varifill_minimum_chunk_size() {
        // Even with everything minimal, chunk should be at least 1024
        let chunk = calculate_varifill_chunk(1.0, 100, 1_000_000.0, 1.0);

        assert!(chunk >= 1024, "Minimum chunk size should be 1024");
    }

    #[test]
    fn test_varifill_clamps_multiplier() {
        // Extreme values should be clamped
        let base = 4096;

        // Very empty buffer + fast disk + high speed
        let extreme_high = calculate_varifill_chunk(0.0, base, 100_000_000.0, 4.0);
        // Multiplier would be (0.5 + 1.5) * 2.0 * 4.0 = 16.0, clamped to 4.0
        assert_eq!(extreme_high, base * 4, "Should clamp to 4x base");

        // Very full buffer + slow disk
        let extreme_low = calculate_varifill_chunk(1.0, base, 1_000_000.0, 1.0);
        // Multiplier would be 0.5 * 0.316 * 1.0 = 0.158, clamped to 0.25
        // But then max(1024) kicks in
        assert!(extreme_low >= 1024, "Should respect minimum chunk size");
    }

    #[test]
    fn test_varifill_bandwidth_factor_clamped() {
        // Bandwidth factor should be clamped between 0.5 and 2.0
        let base = 4096;

        // Very high bandwidth (100x baseline)
        let very_high = calculate_varifill_chunk(0.5, base, 1_000_000_000.0, 1.0);
        // sqrt(100) = 10, but clamped to 2.0
        let expected_high = (base as f64 * 1.25 * 2.0) as usize;
        assert_eq!(very_high, expected_high);

        // Very low bandwidth (0.01x baseline)
        let very_low = calculate_varifill_chunk(0.5, base, 100_000.0, 1.0);
        // sqrt(0.01) = 0.1, but clamped to 0.5
        let expected_low = (base as f64 * 1.25 * 0.5) as usize;
        assert_eq!(very_low, expected_low);
    }

    // =========================================================================
    // Edge case / robustness tests
    // =========================================================================

    #[test]
    fn test_varifill_buffer_fill_over_one() {
        // Buffer fill > 1.0 (shouldn't happen, but test robustness)
        let chunk = calculate_varifill_chunk(1.5, 4096, 10_000_000.0, 1.0);

        // urgency = 1.0 - 1.5 = -0.5
        // multiplier = (0.5 + (-0.5)*1.5) * 1.0 * 1.0 = -0.25, clamped to 0.25
        assert!(chunk >= 1024, "Should still respect minimum");
    }

    #[test]
    fn test_varifill_buffer_fill_negative() {
        // Buffer fill < 0 (shouldn't happen, but test robustness)
        let chunk = calculate_varifill_chunk(-0.5, 4096, 10_000_000.0, 1.0);

        // urgency = 1.0 - (-0.5) = 1.5
        // multiplier = (0.5 + 1.5*1.5) * 1.0 * 1.0 = 2.75
        assert!(chunk > 4096, "Should increase chunk for negative fill");
    }

    #[test]
    fn test_varifill_nan_buffer_fill() {
        // NaN buffer_fill - should not crash, clamp handles it
        let chunk = calculate_varifill_chunk(f32::NAN, 4096, 10_000_000.0, 1.0);
        assert!(chunk >= 1024, "Should respect minimum even with NaN");
    }

    #[test]
    fn test_varifill_infinity_bandwidth() {
        // Infinite bandwidth - should be clamped
        let chunk = calculate_varifill_chunk(0.5, 4096, f64::INFINITY, 1.0);
        // sqrt(inf) = inf, but clamped to 2.0
        let expected = (4096.0 * 1.25 * 2.0) as usize;
        assert_eq!(chunk, expected);
    }

    #[test]
    fn test_varifill_negative_bandwidth() {
        // Negative bandwidth (invalid) - should use default factor 1.0
        let chunk = calculate_varifill_chunk(0.5, 4096, -1000.0, 1.0);
        let normal = calculate_varifill_chunk(0.5, 4096, 10_000_000.0, 1.0);
        // Negative is not > 0, so uses factor 1.0
        assert_eq!(chunk, normal);
    }

    // =========================================================================
    // fill_buffer_forward tests (for loop-aware refill logic verification)
    // =========================================================================

    fn make_test_wave(samples: &[(f32, f32)]) -> Wave {
        let mut wave = Wave::new(2, 48000.0);
        for (l, r) in samples {
            wave.push((*l, *r));
        }
        wave
    }

    #[test]
    fn test_fill_buffer_forward_basic() {
        // Create a simple wave: 0.1, 0.2, 0.3, 0.4
        let wave = make_test_wave(&[(0.1, 0.1), (0.2, 0.2), (0.3, 0.3), (0.4, 0.4)]);
        let mut buffer = Vec::new();

        fill_buffer_forward(&wave, 0, 3, 2, &mut buffer);

        assert_eq!(buffer.len(), 3);
        assert_eq!(buffer[0], (0.1, 0.1));
        assert_eq!(buffer[1], (0.2, 0.2));
        assert_eq!(buffer[2], (0.3, 0.3));
    }

    #[test]
    fn test_fill_buffer_forward_past_end_pads_zeros() {
        let wave = make_test_wave(&[(0.1, 0.1), (0.2, 0.2)]);
        let mut buffer = Vec::new();

        fill_buffer_forward(&wave, 1, 4, 2, &mut buffer);

        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer[0], (0.2, 0.2)); // Last valid sample
        assert_eq!(buffer[1], (0.0, 0.0)); // Past end - zeros
        assert_eq!(buffer[2], (0.0, 0.0));
        assert_eq!(buffer[3], (0.0, 0.0));
    }

    // =========================================================================
    // Loop-aware refill position calculation tests
    // =========================================================================

    #[test]
    fn test_loop_wrap_position_calculation() {
        // Test the wrapping logic used in refill_forward_loop_aware
        let loop_start = 100usize;
        let loop_end = 200usize;
        let loop_len = loop_end - loop_start;

        // Position at exactly loop_end should wrap to loop_start
        let pos = 200usize;
        let wrapped = if pos >= loop_end {
            loop_start + ((pos - loop_start) % loop_len)
        } else {
            pos
        };
        assert_eq!(wrapped, loop_start);

        // Position past loop_end should wrap correctly
        let pos = 250usize;
        let wrapped = if pos >= loop_end {
            loop_start + ((pos - loop_start) % loop_len)
        } else {
            pos
        };
        // 250 - 100 = 150, 150 % 100 = 50, 100 + 50 = 150
        assert_eq!(wrapped, 150);

        // Position 2 full loops past should wrap back
        let pos = 300usize;
        let wrapped = if pos >= loop_end {
            loop_start + ((pos - loop_start) % loop_len)
        } else {
            pos
        };
        // 300 - 100 = 200, 200 % 100 = 0, 100 + 0 = 100
        assert_eq!(wrapped, loop_start);
    }

    #[test]
    fn test_loop_wrap_does_not_affect_position_before_loop_end() {
        let loop_start = 100usize;
        let loop_end = 200usize;

        // Position before loop_end should not be affected
        for pos in [100, 150, 199] {
            let wrapped = if pos >= loop_end {
                loop_start + ((pos - loop_start) % (loop_end - loop_start))
            } else {
                pos
            };
            assert_eq!(wrapped, pos, "Position {} should not be wrapped", pos);
        }
    }
}
