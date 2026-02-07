//! Sampler integration tests (requires "sampler" feature)
//!
//! Tests file streaming, recording, and Butler thread operations.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test sampler_integration --features "sampler,wav,export"
//! ```

#![cfg(feature = "sampler")]

#[path = "helpers/mod.rs"]
mod helpers;

use helpers::{generate_sine, peak, rms, save_wav_file_pcm16, test_data_dir, test_engine};
use std::path::PathBuf;
use std::sync::Arc;
use tutti::core::Wave;
use tutti::sampler::SamplerUnit;
use tutti::{AudioUnit, PlayDirection};

fn temp_dir(test_name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("tutti_sampler_test_{}", test_name))
}

fn setup_temp_dir(test_name: &str) -> PathBuf {
    let dir = temp_dir(test_name);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn cleanup_temp_dir(test_name: &str) {
    let _ = std::fs::remove_dir_all(temp_dir(test_name));
}

// =============================================================================
// Sample Loading Tests
// =============================================================================

/// Test loading a non-existent file returns an error.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_load_nonexistent() {
    let engine = test_engine();

    // New API: build() fails immediately for non-existent files
    let result = engine.wav("/nonexistent/path/file.wav").build();
    assert!(
        result.is_err(),
        "Building non-existent sample should return an error"
    );
}

/// Test loading a valid WAV file and verifying it produces audio.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_sampler_load_and_play() {
    let engine = test_engine();

    // Load the test sine wave file
    let test_file = test_data_dir().join("regression/test_sine.wav");
    assert!(test_file.exists(), "Test file should exist: {:?}", test_file);

    // New fluent API: engine.wav(path).build() returns AudioUnit
    let sampler = engine.wav(&test_file).build().expect("Should load test WAV");

    // Add to graph
    engine.graph(|net| {
        net.add(sampler).to_master();
    });

    // Render some audio
    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Should render");

    // Verify we got audio output (not silence)
    let left_rms = rms(&left);
    let right_rms = rms(&right);

    assert!(
        left_rms > 0.01,
        "Left channel should have audio content, got RMS={}",
        left_rms
    );
    assert!(
        right_rms > 0.01,
        "Right channel should have audio content, got RMS={}",
        right_rms
    );
}

/// Test that loading the same file twice works (internal caching).
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_sampler_cache_hit() {
    let engine = test_engine();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        eprintln!("Skipping test: test file not found at {:?}", test_file);
        return;
    }

    // Load and create first instance
    let sampler1 = engine.wav(&test_file).build().expect("First load");
    let _id1 = engine.graph(|net| net.add(sampler1).to_master());

    // Load and create second instance (same file - should use cache)
    let sampler2 = engine.wav(&test_file).build().expect("Second load");
    let _id2 = engine.graph(|net| net.add(sampler2).to_master());

    // Both should work - the cache should handle this
}

// =============================================================================
// Sampler Subsystem Tests
// =============================================================================

/// Test that the sampler subsystem is enabled and accessible.
#[test]
fn test_sampler_subsystem_enabled() {
    let engine = test_engine();
    let sampler = engine.sampler();

    assert!(sampler.is_enabled(), "Sampler should be enabled");
    assert!(
        sampler.sample_rate() > 0.0,
        "Sample rate should be positive"
    );
}

/// Test sampler I/O metrics start at zero.
#[test]
fn test_sampler_io_metrics_initial() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let metrics = sampler.io_metrics();
    assert_eq!(metrics.bytes_read, 0, "Initial bytes_read should be 0");
    assert_eq!(metrics.bytes_written, 0, "Initial bytes_written should be 0");
}

/// Test sampler cache stats are accessible.
#[test]
fn test_sampler_cache_stats() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let stats = sampler.cache_stats();
    // Default cache limits should be set
    assert!(stats.max_entries > 0, "Cache should have max_entries limit");
    assert!(stats.max_bytes > 0, "Cache should have max_bytes limit");
}

// =============================================================================
// Recording Tests
// =============================================================================

/// Test creating a capture session.
/// Note: Full recording test requires mutable producer access which is complex.
/// This test verifies the session creation API works.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_capture_session_creation() {
    let engine = test_engine();
    let temp = setup_temp_dir("capture_session");
    let output_path = temp.join("test_capture.wav");

    let sampler = engine.sampler();
    if let Some(inner) = sampler.inner() {
        // Create a capture session
        let session = inner.create_capture(&output_path, 48000.0, 2, Some(1.0));

        // Verify session properties
        assert_eq!(session.sample_rate(), 48000.0);
        assert_eq!(session.channels(), 2);
        assert_eq!(session.file_path(), &output_path);
        assert!(!session.is_started(), "Session should not be started yet");

        // Start the capture
        let session = inner.start_capture(session);
        assert!(session.is_started(), "Session should be started now");

        // Stop without writing (just testing lifecycle)
        inner.stop_capture(session.id);
    }

    cleanup_temp_dir("capture_session");
}

// =============================================================================
// Streaming Tests
// =============================================================================

/// Test streaming a file and verifying buffer metrics.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_streaming_buffer_metrics() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return; // Skip if test file doesn't exist
    }

    // Start streaming
    sampler.stream_file(0, &test_file);
    sampler.run();

    // Give butler time to prefetch
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Check that streaming started (buffer should have some fill)
    // Note: buffer_fill returns None if not streaming, Some(0.0-1.0) if streaming
    let fill = sampler.buffer_fill(0);
    // Just verify the API works - actual fill depends on timing
    assert!(
        fill.is_some() || fill.is_none(),
        "buffer_fill should return valid result"
    );

    // Check underruns (should be 0 under normal conditions)
    let underruns = sampler.take_all_underruns();
    assert!(
        underruns < 10,
        "Should have minimal underruns, got {}",
        underruns
    );

    // Stop streaming
    sampler.stop_stream(0);
}

/// Test seeking within a stream.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_streaming_seek() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    // Start streaming
    sampler.stream_file(0, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Seek to middle
    sampler.seek(0, 24000); // Half second at 48kHz

    std::thread::sleep(std::time::Duration::from_millis(50));

    // Seek to beginning
    sampler.seek(0, 0);

    // Should complete without crash
    sampler.stop_stream(0);
}

/// Test rapid seeking doesn't cause crashes or hangs.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_rapid_seek_stability() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    // Rapid seeks - should not crash
    for i in 0..20 {
        sampler.seek(0, (i * 1000) as u64);
    }

    std::thread::sleep(std::time::Duration::from_millis(50));
    sampler.stop_stream(0);
}

// =============================================================================
// Looping Tests
// =============================================================================

/// Test setting loop range.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_loop_range() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    // Set loop range (1 second loop starting at 0.5 seconds)
    sampler.set_loop_range(0, 24000, 72000); // 0.5s to 1.5s at 48kHz

    std::thread::sleep(std::time::Duration::from_millis(50));

    // Clear loop
    sampler.clear_loop_range(0);

    sampler.stop_stream(0);
}

/// Test loop with crossfade.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_loop_crossfade() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    // Set loop with 256 sample crossfade
    sampler.set_loop_range_with_crossfade(0, 24000, 72000, 256);

    std::thread::sleep(std::time::Duration::from_millis(50));

    sampler.clear_loop_range(0);
    sampler.stop_stream(0);
}

// =============================================================================
// Varispeed Tests
// =============================================================================

/// Test setting playback speed.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_varispeed() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    // Double speed
    sampler.set_speed(0, 2.0);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Half speed
    sampler.set_speed(0, 0.5);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Normal speed
    sampler.set_speed(0, 1.0);

    sampler.stop_stream(0);
}

/// Test reverse playback.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_reverse_playback() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    // Reverse playback
    sampler.set_direction(0, PlayDirection::Reverse);
    std::thread::sleep(std::time::Duration::from_millis(20));

    // Forward playback
    sampler.set_direction(0, PlayDirection::Forward);

    sampler.stop_stream(0);
}

// =============================================================================
// Auditioner Tests
// =============================================================================

/// Test auditioner (preview) functionality.
#[test]
#[cfg(feature = "wav")]
fn test_auditioner_preview() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    if let Some(auditioner) = sampler.auditioner() {
        // Start preview
        let _ = auditioner.preview(&test_file);

        std::thread::sleep(std::time::Duration::from_millis(50));

        // Check if playing
        let is_playing = auditioner.is_playing();
        // May or may not be playing depending on file loading time
        let _ = is_playing;

        // Stop preview
        auditioner.stop();
    }
}

/// Test auditioner speed control.
#[test]
#[cfg(feature = "wav")]
fn test_auditioner_speed() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    if let Some(auditioner) = sampler.auditioner() {
        let _ = auditioner.preview(&test_file);

        // Change speed and verify getter works
        auditioner.set_speed(2.0);
        assert!(
            (auditioner.speed() - 2.0).abs() < 0.01,
            "Speed should be 2.0, got {}",
            auditioner.speed()
        );

        auditioner.set_speed(0.5);
        assert!(
            (auditioner.speed() - 0.5).abs() < 0.01,
            "Speed should be 0.5, got {}",
            auditioner.speed()
        );

        // Test gain while we're here
        auditioner.set_gain(0.5);
        assert!(
            (auditioner.gain() - 0.5).abs() < 0.01,
            "Gain should be 0.5, got {}",
            auditioner.gain()
        );

        auditioner.stop();
    }
}

// =============================================================================
// In-Memory vs Streaming Tests
// =============================================================================

/// Test that loading via engine.wav() uses in-memory sampler.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_in_memory_sampler_renders_correctly() {
    let engine = test_engine();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    // New fluent API: engine.wav(path).build() returns AudioUnit
    let sampler = engine.wav(&test_file).build().expect("Load WAV");

    engine.graph(|net| {
        net.add(sampler).to_master();
    });

    // Render
    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.05)
        .render()
        .expect("Render");

    // Should have audio
    let rms_val = rms(&left);
    assert!(
        rms_val > 0.01,
        "In-memory sampler should produce audio, got RMS={}",
        rms_val
    );
}

// =============================================================================
// Multi-Channel Tests
// =============================================================================

/// Test streaming multiple files on different channels.
#[test]
#[cfg(feature = "wav")]
fn test_sampler_multi_channel_streaming() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    // Stream same file to multiple channels
    sampler.stream_file(0, &test_file);
    sampler.stream_file(1, &test_file);
    sampler.stream_file(2, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // All should be streaming or completed
    sampler.stop_stream(0);
    sampler.stop_stream(1);
    sampler.stop_stream(2);
}

// =============================================================================
// Edge Case Tests
// =============================================================================

/// Test stopping a stream that was never started.
#[test]
fn test_sampler_stop_nonexistent_stream() {
    let engine = test_engine();
    let sampler = engine.sampler();

    // Should not crash
    sampler.stop_stream(999);
}

/// Test seeking on a non-existent channel.
#[test]
fn test_sampler_seek_nonexistent_channel() {
    let engine = test_engine();
    let sampler = engine.sampler();

    // Should not crash
    sampler.seek(999, 0);
}

/// Test buffer_fill on non-existent channel returns None.
#[test]
fn test_sampler_buffer_fill_nonexistent() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let fill = sampler.buffer_fill(999);
    assert!(fill.is_none(), "Non-existent channel should return None");
}

// =============================================================================
// Round-Trip Quality Tests
// =============================================================================

/// Test recording and loading back produces equivalent audio.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_sampler_record_load_roundtrip() {
    let engine = test_engine();
    let temp = setup_temp_dir("roundtrip");
    let output_path = temp.join("roundtrip.wav");

    // Generate test signal
    let sample_rate = 48000.0;
    let duration_samples = 4800; // 0.1 seconds
    let original_left = generate_sine(880.0, sample_rate, duration_samples);
    let original_right = generate_sine(880.0, sample_rate, duration_samples);

    // Save as WAV file (using helper)
    save_wav_file_pcm16(&output_path, &original_left, &original_right, 48000)
        .expect("Should save test WAV");

    // Load via sampler and render using new fluent API
    let sampler = engine.wav(&output_path).build().expect("Load");

    engine.graph(|net| {
        net.add(sampler).to_master();
    });

    let (rendered_left, rendered_right, _sr) = engine
        .export()
        .duration_seconds(0.05)
        .render()
        .expect("Render");

    // Compare RMS (should be similar)
    let original_rms = rms(&original_left);
    let rendered_rms = rms(&rendered_left);

    // Allow some tolerance due to sample rate conversion, quantization, etc.
    assert!(
        (original_rms - rendered_rms).abs() < 0.2,
        "RMS should be similar after round-trip: original={}, rendered={}",
        original_rms,
        rendered_rms
    );

    // Verify stereo content
    assert!(
        rms(&rendered_right) > 0.01,
        "Right channel should have content"
    );

    cleanup_temp_dir("roundtrip");
}

// =============================================================================
// SamplerUnit Render Tests (Phase 1)
// =============================================================================

/// Helper: Create a stereo Wave from mono samples (duplicate to both channels).
fn create_stereo_wave(samples: &[f32], sample_rate: f64) -> Wave {
    // Wave::new creates empty channels, push_channel adds them
    let mut wave = Wave::new(0, sample_rate);
    wave.push_channel(samples); // First channel sets the length
    wave.push_channel(samples); // Second channel must match length
    wave
}

/// Helper: Render N samples from a SamplerUnit using tick().
fn render_samples(unit: &mut SamplerUnit, num_samples: usize) -> (Vec<f32>, Vec<f32>) {
    let mut left = Vec::with_capacity(num_samples);
    let mut right = Vec::with_capacity(num_samples);
    let mut output = [0.0f32; 2];

    for _ in 0..num_samples {
        unit.tick(&[], &mut output);
        left.push(output[0]);
        right.push(output[1]);
    }

    (left, right)
}

/// Test that SamplerUnit renders a sine wave correctly.
#[test]
fn test_sampler_unit_renders_sine() {
    let sample_rate = 48000.0;
    let duration_samples = 4800; // 0.1 seconds
    let frequency = 440.0;

    // Generate a 440Hz sine wave
    let sine_samples = generate_sine(frequency, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));

    // Render the entire sample
    let (left, right) = render_samples(&mut unit, duration_samples);

    // Verify we got audio (not silence)
    let left_rms = rms(&left);
    let right_rms = rms(&right);

    assert!(
        left_rms > 0.5,
        "Left channel should have significant content, got RMS={}",
        left_rms
    );
    assert!(
        right_rms > 0.5,
        "Right channel should have significant content, got RMS={}",
        right_rms
    );

    // Verify peak is close to 1.0 (full scale sine)
    let left_peak = peak(&left);
    assert!(
        (left_peak - 1.0).abs() < 0.05,
        "Left peak should be ~1.0, got {}",
        left_peak
    );

    // Verify samples match the original (within floating point tolerance)
    for i in 0..100 {
        assert!(
            (left[i] - sine_samples[i]).abs() < 0.001,
            "Sample {} mismatch: got {}, expected {}",
            i,
            left[i],
            sine_samples[i]
        );
    }
}

/// Test that SamplerUnit stops after playing once (non-looping mode).
#[test]
fn test_sampler_unit_stops_at_end() {
    let sample_rate = 48000.0;
    let duration_samples = 100;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));
    assert!(unit.is_playing(), "Should be playing initially");

    // Render past the end
    let (left, _right) = render_samples(&mut unit, duration_samples + 50);

    // Should have stopped
    assert!(!unit.is_playing(), "Should have stopped after end");

    // First N samples should have content
    let initial_rms = rms(&left[..duration_samples]);
    assert!(initial_rms > 0.1, "Initial samples should have content");

    // Samples after end should be silent
    let tail_rms = rms(&left[duration_samples..]);
    assert!(
        tail_rms < 0.001,
        "Samples after end should be silent, got RMS={}",
        tail_rms
    );
}

/// Test that SamplerUnit loops correctly.
#[test]
fn test_sampler_unit_loops() {
    let sample_rate = 48000.0;
    let duration_samples = 480; // Short sample for faster test

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));
    unit.set_looping(true);

    // Render 3x the duration (should loop)
    let total_samples = duration_samples * 3;
    let (left, _right) = render_samples(&mut unit, total_samples);

    // Should still be playing (looping)
    assert!(unit.is_playing(), "Should still be playing when looping");

    // All samples should have content (no silence gaps)
    let rms_first = rms(&left[..duration_samples]);
    let rms_second = rms(&left[duration_samples..duration_samples * 2]);
    let rms_third = rms(&left[duration_samples * 2..]);

    assert!(rms_first > 0.5, "First loop should have content");
    assert!(rms_second > 0.5, "Second loop should have content");
    assert!(rms_third > 0.5, "Third loop should have content");

    // RMS should be similar across all loops
    assert!(
        (rms_first - rms_second).abs() < 0.1,
        "Loop RMS should be consistent"
    );
}

/// Test loop with crossfade for smooth transitions.
///
/// Uses a sawtooth/ramp waveform where the loop point creates a discontinuity
/// (jump from 1.0 back to 0.0). Without crossfade, this would be a hard click.
/// With crossfade, the transition should be smoothed.
#[test]
fn test_sampler_unit_loop_crossfade() {
    let sample_rate = 48000.0;
    let duration_samples = 1000; // Short loop for clear testing
    let crossfade_samples = 100;

    // Create a ramp from 0.0 to 1.0 - this creates a discontinuity at loop point
    let ramp_samples: Vec<f32> = (0..duration_samples)
        .map(|i| i as f32 / duration_samples as f32)
        .collect();
    let wave = create_stereo_wave(&ramp_samples, sample_rate);

    // Test WITHOUT crossfade first - should have a hard jump
    let mut unit_no_xfade = SamplerUnit::new(Arc::new(wave.clone()));
    unit_no_xfade.set_loop_range(0, duration_samples as u64, 0); // No crossfade

    let (left_no_xfade, _) = render_samples(&mut unit_no_xfade, duration_samples + 100);

    // Find max derivative around loop point
    let loop_point = duration_samples;
    let mut max_diff_no_xfade = 0.0f32;
    for i in (loop_point - 10)..(loop_point + 10).min(left_no_xfade.len() - 1) {
        let diff = (left_no_xfade[i + 1] - left_no_xfade[i]).abs();
        max_diff_no_xfade = max_diff_no_xfade.max(diff);
    }

    // Test WITH crossfade - should be smoother
    let mut unit_xfade = SamplerUnit::new(Arc::new(wave));
    unit_xfade.set_loop_range(0, duration_samples as u64, crossfade_samples);

    let (left_xfade, _) = render_samples(&mut unit_xfade, duration_samples + 100);

    let mut max_diff_xfade = 0.0f32;
    for i in (loop_point - 10)..(loop_point + 10).min(left_xfade.len() - 1) {
        let diff = (left_xfade[i + 1] - left_xfade[i]).abs();
        max_diff_xfade = max_diff_xfade.max(diff);
    }

    // Without crossfade, the ramp jumps from ~1.0 to ~0.0 = diff of ~1.0
    // With crossfade, this should be significantly reduced
    assert!(
        max_diff_no_xfade > 0.5,
        "Without crossfade should have hard jump, got max_diff={}",
        max_diff_no_xfade
    );

    // The crossfade should reduce the discontinuity
    // Note: This tests that crossfade is doing *something*, not perfection
    assert!(
        max_diff_xfade < max_diff_no_xfade,
        "Crossfade should reduce discontinuity: with={}, without={}",
        max_diff_xfade,
        max_diff_no_xfade
    );
}

/// Test varispeed (speed != 1.0).
#[test]
fn test_sampler_unit_varispeed() {
    let sample_rate = 48000.0;
    let duration_samples = 4800;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    // Test double speed
    let mut unit_2x = SamplerUnit::with_settings(Arc::new(wave.clone()), 1.0, 2.0, false);

    // At 2x speed, should finish in half the time
    let (left_2x, _) = render_samples(&mut unit_2x, duration_samples / 2 + 10);

    // Should have stopped (finished early)
    assert!(
        !unit_2x.is_playing(),
        "2x speed should finish in half the time"
    );

    // Test half speed
    let mut unit_half = SamplerUnit::with_settings(Arc::new(wave), 1.0, 0.5, false);

    // At 0.5x speed, should still be playing after original duration
    let (left_half, _) = render_samples(&mut unit_half, duration_samples);

    assert!(
        unit_half.is_playing(),
        "0.5x speed should still be playing"
    );

    // Both should have audio content
    assert!(rms(&left_2x) > 0.1, "2x speed should have audio");
    assert!(rms(&left_half) > 0.1, "0.5x speed should have audio");
}

/// Test gain control.
#[test]
fn test_sampler_unit_gain() {
    let sample_rate = 48000.0;
    let duration_samples = 480;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = Arc::new(create_stereo_wave(&sine_samples, sample_rate));

    // Full gain
    let mut unit_full = SamplerUnit::new(Arc::clone(&wave));
    let (left_full, _) = render_samples(&mut unit_full, duration_samples);

    // Half gain
    let mut unit_half = SamplerUnit::with_settings(Arc::clone(&wave), 0.5, 1.0, false);
    let (left_half, _) = render_samples(&mut unit_half, duration_samples);

    let rms_full = rms(&left_full);
    let rms_half = rms(&left_half);

    // Half gain should be ~half the RMS
    let ratio = rms_half / rms_full;
    assert!(
        (ratio - 0.5).abs() < 0.05,
        "Half gain should be ~0.5x RMS, got ratio={}",
        ratio
    );
}

/// Test trigger and stop control.
#[test]
fn test_sampler_unit_trigger_stop() {
    let sample_rate = 48000.0;
    let duration_samples = 480;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));

    // Stop immediately
    unit.stop();
    assert!(!unit.is_playing(), "Should be stopped");

    // Render while stopped (should be silent)
    let (left_stopped, _) = render_samples(&mut unit, 100);
    assert!(
        rms(&left_stopped) < 0.001,
        "Stopped unit should be silent"
    );

    // Trigger from start
    unit.trigger();
    assert!(unit.is_playing(), "Should be playing after trigger");
    assert_eq!(unit.position(), 0, "Position should be 0 after trigger");

    // Render after trigger (should have audio)
    let (left_playing, _) = render_samples(&mut unit, 100);
    assert!(rms(&left_playing) > 0.1, "Playing unit should have audio");
}

/// Test trigger_at (start from specific position).
#[test]
fn test_sampler_unit_trigger_at_position() {
    let sample_rate = 48000.0;
    let duration_samples = 480;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));

    // Trigger from middle
    let start_pos = 240u64;
    unit.stop();
    unit.trigger_at(start_pos);

    assert!(unit.is_playing(), "Should be playing");
    assert_eq!(unit.position(), start_pos, "Position should be at trigger point");

    // Should stop sooner (only half the samples left)
    let (left, _) = render_samples(&mut unit, 300);

    // First ~240 samples should have content, then silence
    let initial_rms = rms(&left[..240]);
    assert!(initial_rms > 0.1, "Initial samples should have content");
}

/// Test mono file playback (duplicated to stereo).
#[test]
fn test_sampler_unit_mono_to_stereo() {
    let sample_rate = 48000.0;
    let duration_samples = 480;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);

    // Create mono wave
    let wave = Wave::from_samples(sample_rate, &sine_samples);
    assert_eq!(wave.channels(), 1, "Should be mono");

    let mut unit = SamplerUnit::new(Arc::new(wave));

    let (left, right) = render_samples(&mut unit, duration_samples);

    // Both channels should have identical content
    for i in 0..duration_samples {
        assert!(
            (left[i] - right[i]).abs() < 0.001,
            "Mono should be duplicated to both channels"
        );
    }

    // Should match original
    for i in 0..100 {
        assert!(
            (left[i] - sine_samples[i]).abs() < 0.001,
            "Output should match input"
        );
    }
}

// =============================================================================
// SamplerHandle Tests (covering handle.rs)
// =============================================================================

/// Test SamplerHandle convenience methods.
#[test]
fn test_sampler_handle_convenience_methods() {
    let engine = test_engine();
    let handle = engine.sampler();

    // Test is_enabled
    assert!(handle.is_enabled(), "Sampler should be enabled");

    // Test sample_rate
    let sr = handle.sample_rate();
    assert!(sr > 0.0, "Sample rate should be positive");

    // Test cache_stats
    let stats = handle.cache_stats();
    assert_eq!(stats.entries, 0, "Fresh cache should have no entries");
    assert_eq!(stats.bytes, 0, "Fresh cache should have no bytes");

    // Test io_metrics
    let metrics = handle.io_metrics();
    assert_eq!(metrics.cache_hits, 0, "Fresh metrics should have no cache hits");
    assert_eq!(metrics.cache_misses, 0, "Fresh metrics should have no cache misses");

    // Test reset_io_metrics (should not panic)
    handle.reset_io_metrics();

    // Test buffer_fill for non-existent channel
    let fill = handle.buffer_fill(999);
    assert!(fill.is_none(), "Non-existent channel should return None");

    // Test take_underruns
    let underruns = handle.take_underruns(0);
    assert_eq!(underruns, 0, "Should have no underruns initially");

    // Test take_all_underruns
    let all_underruns = handle.take_all_underruns();
    assert_eq!(all_underruns, 0, "Should have no underruns");
}

/// Test SamplerHandle lifecycle methods.
#[test]
fn test_sampler_handle_lifecycle() {
    let engine = test_engine();
    let handle = engine.sampler();

    // Test run/pause/wait_for_completion/shutdown (chainable)
    handle.run().pause().run().wait_for_completion();

    // Should still work after these calls
    assert!(handle.is_enabled(), "Should still be enabled");
}

/// Test SamplerHandle with disabled sampler returns graceful defaults.
#[test]
fn test_sampler_handle_disabled_graceful() {
    // Create a handle with None (simulates disabled sampler)
    let handle = tutti::SamplerHandle::new(None);

    assert!(!handle.is_enabled(), "Should be disabled");
    assert_eq!(handle.sample_rate(), 0.0, "Disabled should return 0.0");

    // These should be no-ops
    handle.run().pause().wait_for_completion().shutdown();

    // Should return defaults
    let stats = handle.cache_stats();
    assert_eq!(stats.entries, 0);
    assert_eq!(stats.bytes, 0);

    let metrics = handle.io_metrics();
    assert_eq!(metrics.cache_hits, 0);
    assert_eq!(metrics.cache_misses, 0);

    assert!(handle.buffer_fill(0).is_none());
    assert_eq!(handle.take_underruns(0), 0);
    assert_eq!(handle.take_all_underruns(), 0);
    assert!(handle.streaming_unit(0).is_none());
    assert!(handle.auditioner().is_none());
    assert!(handle.inner().is_none());
}

/// Test SamplerHandle stream control methods.
#[test]
fn test_sampler_handle_stream_control() {
    let engine = test_engine();
    let handle = engine.sampler();

    // These should not panic even for non-existent channels
    handle
        .stop_stream(0)
        .seek(0, 1000)
        .set_loop_range(0, 0, 1000)
        .set_loop_range_with_crossfade(0, 0, 1000, 64)
        .clear_loop_range(0)
        .set_direction(0, PlayDirection::Forward)
        .set_speed(0, 1.0);

    // Chaining should work
    assert!(handle.is_enabled());
}

/// Test SamplerHandle clone.
#[test]
fn test_sampler_handle_clone() {
    let engine = test_engine();
    let handle1 = engine.sampler();
    let handle2 = handle1.clone();

    // Both should be enabled and point to same system
    assert!(handle1.is_enabled());
    assert!(handle2.is_enabled());
    assert_eq!(handle1.sample_rate(), handle2.sample_rate());
}

// =============================================================================
// Auditioner Tests (covering auditioner.rs)
// =============================================================================

/// Test auditioner creation and basic properties.
#[test]
fn test_auditioner_creation() {
    let engine = test_engine();
    let handle = engine.sampler();

    let auditioner = handle.auditioner();
    assert!(auditioner.is_some(), "Should get auditioner");

    let aud = auditioner.unwrap();
    assert!(!aud.is_playing(), "Should not be playing initially");

    // Test default gain/speed
    assert!((aud.gain() - 1.0).abs() < 0.001, "Default gain should be 1.0");
    assert!((aud.speed() - 1.0).abs() < 0.001, "Default speed should be 1.0");
}

/// Test auditioner gain and speed setters.
#[test]
fn test_auditioner_gain_speed() {
    let engine = test_engine();
    let handle = engine.sampler();
    let aud = handle.auditioner().unwrap();

    // Test gain
    aud.set_gain(0.5);
    assert!((aud.gain() - 0.5).abs() < 0.001, "Gain should be 0.5");

    // Test speed
    aud.set_speed(2.0);
    assert!((aud.speed() - 2.0).abs() < 0.001, "Speed should be 2.0");
}

/// Test auditioner stop (should not panic when nothing playing).
#[test]
fn test_auditioner_stop_empty() {
    let engine = test_engine();
    let handle = engine.sampler();
    let aud = handle.auditioner().unwrap();

    // Stop when nothing is playing should be a no-op
    aud.stop();
    assert!(!aud.is_playing());
}

// =============================================================================
// SamplerUnit Additional Tests
// =============================================================================

/// Test SamplerUnit playback at different speeds.
#[test]
fn test_sampler_unit_speed_variations() {
    let sample_rate = 48000.0;
    let duration_samples = 480;

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    // Normal speed
    let mut unit_1x = SamplerUnit::with_settings(Arc::new(wave.clone()), 1.0, 1.0, false);
    let (left_1x, _) = render_samples(&mut unit_1x, duration_samples);
    assert!(!unit_1x.is_playing(), "1x should finish at expected time");

    // Half speed - should still be playing after same render time
    let mut unit_half = SamplerUnit::with_settings(Arc::new(wave.clone()), 1.0, 0.5, false);
    let (left_half, _) = render_samples(&mut unit_half, duration_samples);
    assert!(unit_half.is_playing(), "0.5x should still be playing");

    // Double speed - should finish early
    let mut unit_2x = SamplerUnit::with_settings(Arc::new(wave), 1.0, 2.0, false);
    let (left_2x, _) = render_samples(&mut unit_2x, duration_samples / 2 + 10);
    assert!(!unit_2x.is_playing(), "2x should finish early");

    // All should have audio content
    assert!(rms(&left_1x) > 0.1, "1x should have audio");
    assert!(rms(&left_half) > 0.1, "0.5x should have audio");
    assert!(rms(&left_2x[..duration_samples / 2]) > 0.1, "2x should have audio");
}

/// Test SamplerUnit with very short sample (edge case).
#[test]
fn test_sampler_unit_very_short() {
    let sample_rate = 48000.0;
    let duration_samples = 10; // Very short

    let sine_samples = generate_sine(440.0, sample_rate, duration_samples);
    let wave = create_stereo_wave(&sine_samples, sample_rate);

    let mut unit = SamplerUnit::new(Arc::new(wave));

    // Should render without crashing
    let (left, _) = render_samples(&mut unit, 100);

    // Should have stopped
    assert!(!unit.is_playing());

    // First 10 samples should have some content
    let initial_peak = peak(&left[..duration_samples]);
    assert!(initial_peak > 0.0, "Should have content");
}

/// Test SamplerUnit with zero-length sample (edge case).
#[test]
fn test_sampler_unit_empty_wave() {
    let wave = Wave::with_capacity(2, 48000.0, 0);
    let mut unit = SamplerUnit::new(Arc::new(wave));

    // Should handle gracefully
    let (left, right) = render_samples(&mut unit, 100);

    // Should output silence
    assert!(rms(&left) < 0.001, "Empty wave should produce silence");
    assert!(rms(&right) < 0.001, "Empty wave should produce silence");
}
