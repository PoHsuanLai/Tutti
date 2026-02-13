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
                        // Get samples from loop start to pre-fill the buffer
                        // This prevents underruns while the butler refills asynchronously
                        let file_path = producers[idx].file_path();
                        let prefill_samples = if let Some(wave) =
                            get_wave_from_cache(sample_cache, metrics, file_path)
                        {
                            // Capture the ENTIRE loop region to prevent underruns
                            // This ensures the audio thread always has valid content
                            let loop_end = stream_state
                                .loop_range()
                                .map(|(_, end)| end as usize)
                                .unwrap_or(wave.len());
                            let loop_len = loop_end - loop_start as usize;
                            // Fill buffer with at least one full loop iteration
                            // Limited by ring buffer capacity
                            let prefill_len = loop_len.min(producers[idx].write_space());
                            capture_samples(&wave, loop_start as usize, prefill_len)
                        } else {
                            Vec::new()
                        };

                        // Flush old content and seek to loop start
                        stream_state.flush_buffer();
                        producers[idx].set_file_position(loop_start);

                        // Immediately write pre-captured samples to the buffer
                        if !prefill_samples.is_empty() {
                            let written = producers[idx].write(&prefill_samples);
                            // Advance file position to account for pre-filled samples
                            producers[idx].set_file_position(loop_start + written as u64);
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // capture_samples tests
    // =========================================================================

    fn make_test_wave(samples: &[(f32, f32)]) -> Wave {
        let mut wave = Wave::new(2, 48000.0);
        for (l, r) in samples {
            wave.push((*l, *r));
        }
        wave
    }

    fn make_mono_wave(samples: &[f32]) -> Wave {
        let mut wave = Wave::new(1, 48000.0);
        for s in samples {
            wave.push(*s);
        }
        wave
    }

    #[test]
    fn test_capture_samples_basic() {
        let wave = make_test_wave(&[(1.0, 2.0), (3.0, 4.0), (5.0, 6.0), (7.0, 8.0)]);

        let captured = capture_samples(&wave, 0, 3);

        assert_eq!(captured.len(), 3);
        assert_eq!(captured[0], (1.0, 2.0));
        assert_eq!(captured[1], (3.0, 4.0));
        assert_eq!(captured[2], (5.0, 6.0));
    }

    #[test]
    fn test_capture_samples_with_offset() {
        let wave = make_test_wave(&[(1.0, 2.0), (3.0, 4.0), (5.0, 6.0), (7.0, 8.0)]);

        let captured = capture_samples(&wave, 2, 2);

        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0], (5.0, 6.0));
        assert_eq!(captured[1], (7.0, 8.0));
    }

    #[test]
    fn test_capture_samples_past_end_pads_zeros() {
        let wave = make_test_wave(&[(1.0, 2.0), (3.0, 4.0)]);

        let captured = capture_samples(&wave, 1, 4);

        assert_eq!(captured.len(), 4);
        assert_eq!(captured[0], (3.0, 4.0)); // Valid sample
        assert_eq!(captured[1], (0.0, 0.0)); // Past end - zero
        assert_eq!(captured[2], (0.0, 0.0));
        assert_eq!(captured[3], (0.0, 0.0));
    }

    #[test]
    fn test_capture_samples_empty_request() {
        let wave = make_test_wave(&[(1.0, 2.0)]);

        let captured = capture_samples(&wave, 0, 0);

        assert!(captured.is_empty());
    }

    #[test]
    fn test_capture_samples_mono_duplicates_to_stereo() {
        let wave = make_mono_wave(&[1.0, 2.0, 3.0]);

        let captured = capture_samples(&wave, 0, 3);

        assert_eq!(captured.len(), 3);
        assert_eq!(captured[0], (1.0, 1.0)); // Mono duplicated to stereo
        assert_eq!(captured[1], (2.0, 2.0));
        assert_eq!(captured[2], (3.0, 3.0));
    }

    // =========================================================================
    // calculate_buffer_size tests
    // =========================================================================

    #[test]
    fn test_buffer_size_small_file() {
        // Small file: 1 second at 48kHz = 48000 samples
        // file_size_bytes = 48000 * 2 * 4 = 384000 bytes = 0.37 MB
        // buffer_seconds = min(1.0, 30.0) = 1.0
        let size = calculate_buffer_size(48000, 48000.0);
        assert_eq!(size, 48000); // 1 second buffer
    }

    #[test]
    fn test_buffer_size_medium_file() {
        // Medium file: 100MB = 100 * 1024 * 1024 bytes
        // file_size_bytes = file_length * 2 * 4 = file_length * 8
        // For 100MB: file_length = 100 * 1024 * 1024 / 8 = 13,107,200 samples
        let file_length = 100 * 1024 * 1024 / 8;
        let size = calculate_buffer_size(file_length, 48000.0);

        // 100MB is in 50-200MB range, so buffer_seconds = 10.0
        let expected = (10.0 * 48000.0) as usize;
        assert_eq!(size, expected);
    }

    #[test]
    fn test_buffer_size_large_file() {
        // Large file: 300MB
        let file_length = 300 * 1024 * 1024 / 8;
        let size = calculate_buffer_size(file_length, 48000.0);

        // 300MB is in 200-500MB range, so buffer_seconds = 5.0
        let expected = (5.0 * 48000.0) as usize;
        assert_eq!(size, expected);
    }

    #[test]
    fn test_buffer_size_very_large_file() {
        // Very large file: 1GB
        let file_length = 1024 * 1024 * 1024 / 8;
        let size = calculate_buffer_size(file_length, 48000.0);

        // 1GB > 500MB, so buffer_seconds = 3.0
        let expected = (3.0 * 48000.0) as usize;
        assert_eq!(size, expected);
    }

    #[test]
    fn test_buffer_size_minimum() {
        // Tiny file should still have minimum buffer
        let size = calculate_buffer_size(100, 48000.0);
        assert!(size >= 4096, "Buffer should be at least 4096 samples");
    }

    #[test]
    fn test_buffer_size_small_file_capped_at_30s() {
        // File that would need more than 30 seconds should be capped
        // 60 seconds at 48kHz = 2,880,000 samples
        // file_size = 2,880,000 * 8 = 23MB (< 50MB, so uses file duration)
        // But capped at 30 seconds
        let file_length = 60 * 48000; // 60 seconds
        let size = calculate_buffer_size(file_length, 48000.0);

        let expected = (30.0 * 48000.0) as usize; // Capped at 30s
        assert_eq!(size, expected);
    }

    // =========================================================================
    // capture_fadeout_samples tests (needs mock stream state)
    // =========================================================================

    #[test]
    fn test_capture_fadeout_zero_count() {
        use crate::butler::stream_state::ChannelStreamState;

        let state = ChannelStreamState::default();
        let samples = capture_fadeout_samples(&state, 0);

        assert!(samples.is_empty());
    }

    #[test]
    fn test_capture_fadeout_no_consumer() {
        use crate::butler::stream_state::ChannelStreamState;

        let state = ChannelStreamState::default();
        // No consumer attached
        let samples = capture_fadeout_samples(&state, 100);

        assert!(samples.is_empty());
    }

    // =========================================================================
    // capture_fadein_samples tests
    // =========================================================================

    #[test]
    fn test_capture_fadein_zero_count() {
        let cache = LruCache::new(10, 1024 * 1024);
        let metrics = IOMetrics::new();
        let path = PathBuf::from("nonexistent.wav");

        let samples = capture_fadein_samples(&cache, &metrics, &path, 0, 0);

        assert!(samples.is_empty());
    }

    #[test]
    fn test_capture_fadein_file_not_in_cache() {
        let cache = LruCache::new(10, 1024 * 1024);
        let metrics = IOMetrics::new();
        let path = PathBuf::from("nonexistent.wav");

        let samples = capture_fadein_samples(&cache, &metrics, &path, 0, 100);

        // File not in cache and doesn't exist, so returns empty
        assert!(samples.is_empty());
    }

    // =========================================================================
    // Edge case / potential bug tests
    // =========================================================================

    #[test]
    fn test_capture_samples_empty_wave() {
        // Empty wave with 0 samples - should return zeros
        let wave = Wave::new(2, 48000.0);
        assert_eq!(wave.len(), 0);

        let captured = capture_samples(&wave, 0, 3);

        // Should pad with zeros since wave is empty
        assert_eq!(captured.len(), 3);
        assert_eq!(captured[0], (0.0, 0.0));
        assert_eq!(captured[1], (0.0, 0.0));
        assert_eq!(captured[2], (0.0, 0.0));
    }

    #[test]
    fn test_capture_samples_large_start_no_panic() {
        let wave = make_test_wave(&[(1.0, 2.0), (3.0, 4.0)]);

        // Start way past wave length - should not panic, just return zeros
        let captured = capture_samples(&wave, 1_000_000, 5);

        // All indices are way past wave.len(), so all zeros
        assert_eq!(captured.len(), 5);
        for sample in captured {
            assert_eq!(sample, (0.0, 0.0));
        }
    }
}
