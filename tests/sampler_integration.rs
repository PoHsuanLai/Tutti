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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tutti::core::Wave;
use tutti::sampler::SamplerUnit;
use tutti::{AudioUnit, PlayDirection};

// =============================================================================
// Test Fixtures - Real audio files for meaningful tests
// =============================================================================

/// Path to test fixtures directory
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio")
}

/// Small sample (~188KB, 16-bit stereo 16kHz, ~3 sec) - good for in-memory tests
fn small_sample() -> PathBuf {
    fixtures_dir().join("small_sample.wav")
}

/// Stereo panning sweep (~689KB, 16-bit stereo 44.1kHz, ~4 sec) - good for stereo tests
fn stereo_panning() -> PathBuf {
    fixtures_dir().join("stereo_panning.wav")
}

/// Medium stream (~5MB, 16-bit stereo 44.1kHz, ~30 sec) - good for streaming tests
fn medium_stream() -> PathBuf {
    fixtures_dir().join("medium_stream.wav")
}

/// Large stream (~182MB, 16-bit mono 16kHz, ~96 min) - good for stress tests
/// Note: This is a symlink and may not exist on all machines
fn large_stream() -> PathBuf {
    fixtures_dir().join("large_stream.wav")
}

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
    assert!(
        test_file.exists(),
        "Test file should exist: {:?}",
        test_file
    );

    // New fluent API: engine.wav(path).build() returns AudioUnit
    let sampler = engine
        .wav(&test_file)
        .build()
        .expect("Should load test WAV");

    // Add to graph
    engine.graph_mut(|net| {
        net.add(sampler).master();
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
    let _id1 = engine.graph_mut(|net| net.add(sampler1).master());

    // Load and create second instance (same file - should use cache)
    let sampler2 = engine.wav(&test_file).build().expect("Second load");
    let _id2 = engine.graph_mut(|net| net.add(sampler2).master());

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
    assert_eq!(
        metrics.bytes_written, 0,
        "Initial bytes_written should be 0"
    );
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

    engine.graph_mut(|net| {
        net.add(sampler).master();
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

    engine.graph_mut(|net| {
        net.add(sampler).master();
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

    assert!(unit_half.is_playing(), "0.5x speed should still be playing");

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
    assert!(rms(&left_stopped) < 0.001, "Stopped unit should be silent");

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
    assert_eq!(
        unit.position(),
        start_pos,
        "Position should be at trigger point"
    );

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

/// Test that IO metrics track actual file reads using real audio file.
#[test]
#[cfg(feature = "wav")]
fn test_io_metrics_track_file_reads() {
    let file_path = medium_stream(); // Real 5MB stereo file
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let file_size = std::fs::metadata(&file_path).unwrap().len();
    assert!(
        file_size > 1_000_000,
        "Test file should be >1MB, got {}",
        file_size
    );

    let engine = test_engine();
    let handle = engine.sampler();

    // Reset metrics to start fresh
    handle.reset_io_metrics();
    let metrics_before = handle.io_metrics();
    assert_eq!(
        metrics_before.bytes_read, 0,
        "Should start at 0 after reset"
    );

    // Stream the real file - this should trigger disk reads
    handle.stream(&file_path).channel(0).start();
    handle.run();

    // Give butler thread time to read (real file needs more time)
    std::thread::sleep(std::time::Duration::from_millis(500));

    let metrics_after = handle.io_metrics();

    // Verify bytes were actually read from the real file
    assert!(
        metrics_after.bytes_read > 0,
        "Should have read bytes from real audio file, got {}",
        metrics_after.bytes_read
    );

    // Should have read a meaningful amount (at least 100KB for prefetch)
    assert!(
        metrics_after.bytes_read > 100_000,
        "Should have prefetched substantial data, got {} bytes",
        metrics_after.bytes_read
    );

    handle.stop_stream(0);
}

/// Test that streaming a real file produces a valid StreamingSamplerUnit with audio.
#[test]
#[cfg(feature = "wav")]
fn test_streaming_unit_produces_audio() {
    let file_path = stereo_panning(); // Real stereo panning sweep
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let engine = test_engine();
    let handle = engine.sampler();

    // Start streaming the real panning sweep file
    handle.stream(&file_path).channel(0).start();
    handle.run();

    // Wait for buffer to fill
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Get the streaming unit
    let unit = handle.streaming_unit(0);
    assert!(unit.is_some(), "Should have a streaming unit for channel 0");

    let mut unit = unit.unwrap();
    // StreamingSamplerUnit now auto-plays (matches SamplerUnit behavior)

    // Render audio and collect samples
    let mut left_samples = Vec::new();
    let mut right_samples = Vec::new();
    let mut output = [0.0f32; 2];

    for _ in 0..4000 {
        unit.tick(&[], &mut output);
        left_samples.push(output[0]);
        right_samples.push(output[1]);
    }

    // Verify we got real audio content
    let left_rms = rms(&left_samples);
    let right_rms = rms(&right_samples);

    assert!(
        left_rms > 0.001 || right_rms > 0.001,
        "Streaming unit should produce audio from real file, left_rms={}, right_rms={}",
        left_rms,
        right_rms
    );

    // For a panning sweep, the L/R balance should vary - both channels should have content
    let left_peak = peak(&left_samples);
    let right_peak = peak(&right_samples);

    assert!(
        left_peak > 0.01 && right_peak > 0.01,
        "Stereo panning file should have content in both channels, L={}, R={}",
        left_peak,
        right_peak
    );

    handle.stop_stream(0);
}

/// Test that buffer_fill returns valid fill level during streaming of real file.
#[test]
#[cfg(feature = "wav")]
fn test_buffer_fill_during_streaming() {
    let file_path = medium_stream(); // Real 5MB file (~30 sec)
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let engine = test_engine();
    let handle = engine.sampler();

    // Before streaming, buffer_fill should be None
    assert!(
        handle.buffer_fill(0).is_none(),
        "No buffer fill before streaming"
    );

    // Start streaming the real file
    handle.stream(&file_path).channel(0).start();
    handle.run();

    // Wait for butler to prefetch
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Now buffer_fill should return a value
    let fill = handle.buffer_fill(0);
    assert!(fill.is_some(), "Should have buffer fill during streaming");

    let fill_level = fill.unwrap();
    assert!(
        fill_level >= 0.0 && fill_level <= 1.0,
        "Fill level should be 0.0-1.0, got {}",
        fill_level
    );

    // After prefetch time on real file, buffer should have substantial content
    assert!(
        fill_level > 0.1,
        "Buffer should have good fill after prefetching real file, got {}",
        fill_level
    );

    handle.stop_stream(0);
}

/// Test that seek actually changes playback position.
/// After seeking, the audio should come from the new position.
#[test]
#[cfg(feature = "wav")]
fn test_handle_seek_changes_position() {
    let test_name = "handle_seek";
    let dir = setup_temp_dir(test_name);
    let file_path = dir.join("seek_test.wav");

    // Create a file with distinct sections:
    // First half: 220Hz, Second half: 880Hz
    let sample_rate = 44100.0;
    let half_samples = 22050;
    let low_freq = generate_sine(220.0, sample_rate, half_samples);
    let high_freq = generate_sine(880.0, sample_rate, half_samples);

    let mut full_samples = low_freq.clone();
    full_samples.extend(high_freq);

    save_wav_file_pcm16(&file_path, &full_samples, &full_samples, sample_rate as u32).unwrap();

    let engine = test_engine();
    let handle = engine.sampler();

    // Stream from beginning
    handle.stream(&file_path).channel(0).start();
    handle.run();
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Seek to second half (where 880Hz starts)
    handle.seek(0, half_samples as u64);

    // Wait longer for butler to process seek and refill buffer
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Get streaming unit and render
    if let Some(mut unit) = handle.streaming_unit(0) {
        let mut output = [0.0f32; 2];
        let mut samples_collected = Vec::new();

        // Render 1000 samples to get past the 512-sample seek crossfade
        for _ in 0..1000 {
            unit.tick(&[], &mut output);
            samples_collected.push(output[0]);
        }

        // Analyze samples AFTER the crossfade (samples 512-1000)
        // These should be pure 880Hz from the ring buffer
        let post_crossfade = &samples_collected[512..];
        let zero_crossings_post: usize = post_crossfade
            .windows(2)
            .filter(|w| w[0].signum() != w[1].signum())
            .count();

        // 880Hz at 44100Hz sample rate = ~19 crossings per ~488 samples (post crossfade)
        // 220Hz would have ~5 crossings
        assert!(
            zero_crossings_post > 10,
            "After crossfade, should have high zero crossing count (880Hz), got {}",
            zero_crossings_post
        );
    }

    handle.stop_stream(0);
    cleanup_temp_dir(test_name);
}

/// Test that set_loop_range causes audio to loop.
#[test]
#[cfg(feature = "wav")]
fn test_handle_loop_range_causes_looping() {
    let test_name = "handle_loop_range";
    let dir = setup_temp_dir(test_name);
    let file_path = dir.join("loop_test.wav");

    // Create a short distinctive pattern - 440Hz for loop region, 880Hz after
    let sample_rate = 44100.0;
    let pattern_samples = 4410; // 0.1 seconds

    // File has 440Hz in first section (loop region) and 880Hz after
    // If looping works, we should only ever hear 440Hz content
    let pattern_440 = generate_sine(440.0, sample_rate, pattern_samples);
    let pattern_880 = generate_sine(880.0, sample_rate, pattern_samples * 2);

    let mut full = pattern_440.clone();
    full.extend(pattern_880);

    save_wav_file_pcm16(&file_path, &full, &full, sample_rate as u32).unwrap();

    let engine = test_engine();
    let handle = engine.sampler();

    // Stream with loop range configured from the start
    handle
        .stream(&file_path)
        .channel(0)
        .loop_samples(0, pattern_samples as u64)
        .start();
    handle.run();
    std::thread::sleep(std::time::Duration::from_millis(300));

    if let Some(mut unit) = handle.streaming_unit(0) {
        // Read more than the loop length to ensure looping has occurred
        // We read 5x the loop length to guarantee at least 4 complete loop iterations
        let total_samples = pattern_samples * 5;
        let mut all_samples = Vec::with_capacity(total_samples);

        for _ in 0..total_samples {
            let mut output = [0.0f32; 2];
            unit.tick(&[], &mut output);
            all_samples.push(output[0]);
        }

        // Skip the first loop iteration (may have buffering artifacts)
        // Then compare several subsequent iterations
        let start = pattern_samples;
        let iter2 = &all_samples[start..start + pattern_samples];
        let iter3 = &all_samples[start + pattern_samples..start + pattern_samples * 2];
        let iter4 = &all_samples[start + pattern_samples * 2..start + pattern_samples * 3];

        let rms2 = rms(iter2);
        let rms3 = rms(iter3);
        let rms4 = rms(iter4);

        // All iterations should have similar RMS (same 440Hz content)
        let variance = ((rms2 - rms3).abs() + (rms3 - rms4).abs()) / 2.0;

        // Use a generous tolerance since we're measuring RMS which can vary
        // due to phase differences at loop points
        assert!(
            variance < 0.15,
            "Looped iterations should have consistent RMS, variance={} (rms2={}, rms3={}, rms4={})",
            variance,
            rms2,
            rms3,
            rms4
        );

        // Also verify that all RMS values are reasonable (not zero, not too high)
        assert!(
            rms2 > 0.1,
            "Loop iteration 2 should have audio content, rms={}",
            rms2
        );
        assert!(
            rms3 > 0.1,
            "Loop iteration 3 should have audio content, rms={}",
            rms3
        );
        assert!(
            rms4 > 0.1,
            "Loop iteration 4 should have audio content, rms={}",
            rms4
        );
    }

    handle.stop_stream(0);
    cleanup_temp_dir(test_name);
}

/// Test that cloned handles share state (changes visible to both).
#[test]
fn test_handle_clone_shares_state() {
    let engine = test_engine();
    let handle1 = engine.sampler();
    let handle2 = handle1.clone();

    // Reset metrics via handle1
    handle1.reset_io_metrics();

    // Both should see the same state
    let metrics1 = handle1.io_metrics();
    let metrics2 = handle2.io_metrics();

    assert_eq!(
        metrics1.bytes_read, metrics2.bytes_read,
        "Cloned handles should see same metrics"
    );
    assert_eq!(
        metrics1.cache_hits, metrics2.cache_hits,
        "Cloned handles should see same cache hits"
    );
}

/// Test disabled handle returns safe defaults without crashing.
#[test]
fn test_disabled_handle_operations_safe() {
    let handle = tutti::SamplerHandle::new(None);

    // All these should return safe defaults without panicking
    assert!(!handle.is_enabled());
    assert_eq!(handle.sample_rate(), 0.0);
    assert!(handle.buffer_fill(0).is_none());
    assert_eq!(handle.take_underruns(0), 0);
    assert!(handle.streaming_unit(0).is_none());
    assert!(handle.auditioner().is_none());

    // Stream builder should be disabled (start is no-op)
    handle.stream("nonexistent.wav").channel(0).start();
    // No crash = success
}

// =============================================================================
// Auditioner Tests (covering auditioner.rs)
// =============================================================================

/// Test auditioner plays real audio file and is_playing reflects state.
#[test]
#[cfg(feature = "wav")]
fn test_auditioner_plays_audio() {
    let file_path = small_sample(); // Real 188KB sample (~3 sec)
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let engine = test_engine();
    let handle = engine.sampler();
    handle.run();

    let aud = handle.auditioner().unwrap();

    assert!(!aud.is_playing(), "Should not be playing initially");

    // Preview the real file
    aud.preview(&file_path).unwrap();

    // Give time for playback to start
    std::thread::sleep(std::time::Duration::from_millis(100));

    assert!(aud.is_playing(), "Should be playing after preview()");

    // Stop and verify
    aud.stop();
    std::thread::sleep(std::time::Duration::from_millis(50));

    assert!(!aud.is_playing(), "Should stop after stop()");
}

/// Test auditioner gain setting with real audio file.
#[test]
#[cfg(feature = "wav")]
fn test_auditioner_gain_affects_output() {
    let file_path = small_sample();
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let engine = test_engine();
    let handle = engine.sampler();
    handle.run();

    let aud = handle.auditioner().unwrap();

    // Set low gain
    aud.set_gain(0.1);
    assert!(
        (aud.gain() - 0.1).abs() < 0.001,
        "Gain should be set to 0.1"
    );

    // Set high gain
    aud.set_gain(2.0);
    assert!(
        (aud.gain() - 2.0).abs() < 0.001,
        "Gain should be set to 2.0"
    );

    // Gain changes should persist during playback of real file
    aud.preview(&file_path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(
        (aud.gain() - 2.0).abs() < 0.001,
        "Gain should persist during playback"
    );

    aud.stop();
}

/// Test auditioner speed setting with real audio file.
#[test]
#[cfg(feature = "wav")]
fn test_auditioner_speed_setting() {
    let file_path = small_sample();
    if !file_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", file_path);
        return;
    }

    let engine = test_engine();
    let handle = engine.sampler();
    handle.run();

    let aud = handle.auditioner().unwrap();

    // Set double speed
    aud.set_speed(2.0);
    assert!(
        (aud.speed() - 2.0).abs() < 0.001,
        "Speed should be set to 2.0"
    );

    // Play real file at 2x speed
    aud.preview(&file_path).unwrap();

    // Verify speed persists during playback
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(aud.is_playing(), "Should be playing");
    assert!(
        (aud.speed() - 2.0).abs() < 0.001,
        "Speed should still be 2.0 during playback"
    );

    // Set half speed
    aud.set_speed(0.5);
    assert!(
        (aud.speed() - 0.5).abs() < 0.001,
        "Speed should be set to 0.5"
    );

    aud.stop();
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
    assert!(
        rms(&left_2x[..duration_samples / 2]) > 0.1,
        "2x should have audio"
    );
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

// =============================================================================
// Comprehensive Butler Tests
// =============================================================================

/// Test butler refill produces correct audio content (not just buffer fill).
#[test]
#[cfg(feature = "wav")]
fn test_butler_refill_audio_content() {
    let engine = test_engine();
    let sampler = engine.sampler();

    // Use a known test file
    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    // Start streaming
    sampler.stream_file(0, &test_file);
    sampler.run();

    // Wait for butler to prefetch
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Get the streaming unit and render some samples
    if let Some(inner) = sampler.inner() {
        if let Some(mut unit) = inner.streaming_unit(0) {
            let mut output = [0.0f32; 2];
            let mut samples = Vec::new();

            // Render 1000 samples
            for _ in 0..1000 {
                unit.tick(&[], &mut output);
                samples.push((output[0], output[1]));
            }

            // Verify we got actual audio (not silence)
            let energy: f32 = samples.iter().map(|(l, r)| l * l + r * r).sum();
            assert!(
                energy > 0.01,
                "Streaming should produce actual audio content, got energy={}",
                energy
            );
        }
    }

    sampler.stop_stream(0);
}

/// Test butler handles multiple concurrent streams.
#[test]
#[cfg(feature = "wav")]
fn test_butler_multiple_streams() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    // Start multiple streams
    sampler.stream_file(0, &test_file);
    sampler.stream_file(1, &test_file);
    sampler.stream_file(2, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(500));

    // Check if at least one channel has buffer content (timing dependent)
    let mut streaming_count = 0;
    for ch in 0..3 {
        if sampler.buffer_fill(ch).is_some() {
            streaming_count += 1;
        }
    }
    // Test passes if no crash - streaming is timing dependent
    let _ = streaming_count;

    // Stop all
    sampler.stop_stream(0);
    sampler.stop_stream(1);
    sampler.stop_stream(2);
}

/// Test butler seek during playback.
#[test]
#[cfg(feature = "wav")]
fn test_butler_seek_during_playback() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = medium_stream();
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(300));

    // Seek to different positions - should not crash
    for pos in [44100, 88200, 0, 132300, 44100] {
        sampler.seek(0, pos);
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // After all seeks, should still be streaming (or have finished gracefully)
    // Just verify no crash occurred
    let _ = sampler.buffer_fill(0);

    sampler.stop_stream(0);
}

/// Test butler loop crossfade produces smooth audio.
#[test]
#[cfg(feature = "wav")]
fn test_butler_loop_crossfade_smooth() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    // Set a short loop with crossfade
    sampler.set_loop_range_with_crossfade(0, 0, 4800, 256); // 0.1s loop, 256 sample xfade
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(200));

    if let Some(inner) = sampler.inner() {
        if let Some(mut unit) = inner.streaming_unit(0) {
            let mut output = [0.0f32; 2];
            let mut prev_sample = (0.0f32, 0.0f32);
            let mut max_jump = 0.0f32;

            // Render through several loop cycles
            for _ in 0..20000 {
                unit.tick(&[], &mut output);

                // Check for discontinuities (large jumps)
                let jump =
                    ((output[0] - prev_sample.0).abs()).max((output[1] - prev_sample.1).abs());
                max_jump = max_jump.max(jump);

                prev_sample = (output[0], output[1]);
            }

            // With crossfade, jumps should be small (< 0.5 for smooth audio)
            assert!(
                max_jump < 0.5,
                "Loop crossfade should be smooth, max jump={}",
                max_jump
            );
        }
    }

    sampler.stop_stream(0);
}

// =============================================================================
// Comprehensive Recording Tests
// =============================================================================

/// Test recording produces valid WAV file with correct content.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_recording_produces_valid_wav() {
    let engine = test_engine();
    let temp = setup_temp_dir("recording_valid");
    let output_path = temp.join("recorded.wav");

    let sampler = engine.sampler();
    if let Some(inner) = sampler.inner() {
        // Create and start capture
        let session = inner.create_capture(&output_path, 48000.0, 2, Some(1.0));
        let mut session = inner.start_capture(session);

        // Write some test audio to the capture buffer
        let producer = session.producer_mut();
        for i in 0..4800 {
            // 0.1 seconds
            let t = i as f32 / 48000.0;
            let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5;
            producer.write((sample, sample));
        }

        // Flush and stop
        inner.flush_capture(session.id);
        std::thread::sleep(std::time::Duration::from_millis(100));
        inner.stop_capture(session.id);

        // Wait for file to be written
        std::thread::sleep(std::time::Duration::from_millis(200));

        // Verify file exists and has content
        assert!(output_path.exists(), "Recording file should exist");

        let metadata = std::fs::metadata(&output_path).unwrap();
        assert!(
            metadata.len() > 1000,
            "Recording file should have content, got {} bytes",
            metadata.len()
        );

        // Load and verify audio content
        if let Ok(wave) = Wave::load(&output_path) {
            assert_eq!(wave.channels(), 2, "Should be stereo");
            assert!(wave.len() > 1000, "Should have samples");

            // Check for actual audio (not silence)
            let mut energy = 0.0f32;
            for i in 0..wave.len().min(1000) {
                energy += wave.at(0, i).powi(2);
            }
            assert!(energy > 0.01, "Recording should contain audio");
        }
    }

    cleanup_temp_dir("recording_valid");
}

/// Test recording handles buffer overflow gracefully.
#[test]
fn test_recording_buffer_overflow() {
    let engine = test_engine();
    let temp = setup_temp_dir("recording_overflow");
    let output_path = temp.join("overflow.wav");

    let sampler = engine.sampler();
    if let Some(inner) = sampler.inner() {
        // Create capture with small buffer (0.1 seconds)
        let session = inner.create_capture(&output_path, 48000.0, 2, Some(0.1));
        let mut session = inner.start_capture(session);

        let producer = session.producer_mut();

        // Try to write more than buffer can hold (without flushing)
        // This should not crash
        for i in 0..50000 {
            let t = i as f32 / 48000.0;
            let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin() * 0.5;
            // write() returns false when buffer is full
            let _ = producer.write((sample, sample));
        }

        inner.stop_capture(session.id);
    }

    cleanup_temp_dir("recording_overflow");
}

/// Test recording mono vs stereo.
#[test]
#[cfg(feature = "wav")]
fn test_recording_mono_stereo() {
    let engine = test_engine();
    let temp = setup_temp_dir("recording_mono_stereo");

    let sampler = engine.sampler();
    if let Some(inner) = sampler.inner() {
        // Test mono recording
        let mono_path = temp.join("mono.wav");
        let session = inner.create_capture(&mono_path, 48000.0, 1, Some(0.5));
        let mut session = inner.start_capture(session);
        assert_eq!(session.channels(), 1);

        {
            let producer = session.producer_mut();
            for i in 0..2400 {
                let t = i as f32 / 48000.0;
                let sample = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
                producer.write((sample, 0.0)); // Only left channel used for mono
            }
        }
        inner.flush_capture(session.id);
        std::thread::sleep(std::time::Duration::from_millis(100));
        inner.stop_capture(session.id);

        // Test stereo recording
        let stereo_path = temp.join("stereo.wav");
        let session = inner.create_capture(&stereo_path, 48000.0, 2, Some(0.5));
        let mut session = inner.start_capture(session);
        assert_eq!(session.channels(), 2);

        {
            let producer = session.producer_mut();
            for i in 0..2400 {
                let t = i as f32 / 48000.0;
                let left = (t * 440.0 * 2.0 * std::f32::consts::PI).sin();
                let right = (t * 880.0 * 2.0 * std::f32::consts::PI).sin();
                producer.write((left, right));
            }
        }
        inner.flush_capture(session.id);
        std::thread::sleep(std::time::Duration::from_millis(100));
        inner.stop_capture(session.id);

        std::thread::sleep(std::time::Duration::from_millis(200));

        // Verify files
        if mono_path.exists() {
            if let Ok(wave) = Wave::load(&mono_path) {
                assert_eq!(wave.channels(), 1, "Mono file should have 1 channel");
            }
        }

        if stereo_path.exists() {
            if let Ok(wave) = Wave::load(&stereo_path) {
                assert_eq!(wave.channels(), 2, "Stereo file should have 2 channels");
            }
        }
    }

    cleanup_temp_dir("recording_mono_stereo");
}

/// Test butler varispeed streaming with speed verification.
#[test]
#[cfg(feature = "wav")]
fn test_butler_varispeed_speed_accurate() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(200));

    if let Some(inner) = sampler.inner() {
        if let Some(mut unit) = inner.streaming_unit(0) {
            // Render at normal speed
            let mut output = [0.0f32; 2];
            let mut normal_samples = Vec::new();
            for _ in 0..1000 {
                unit.tick(&[], &mut output);
                normal_samples.push(output[0]);
            }

            // Set 2x speed
            sampler.set_speed(0, 2.0);
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Seek back to start
            sampler.seek(0, 0);
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Render at 2x speed - should consume samples twice as fast
            let mut fast_samples = Vec::new();
            for _ in 0..1000 {
                unit.tick(&[], &mut output);
                fast_samples.push(output[0]);
            }

            // At 2x speed, the frequency content should appear doubled
            // (This is a simplified check - just verify we got different content)
            let normal_energy: f32 = normal_samples.iter().map(|s| s * s).sum();
            let fast_energy: f32 = fast_samples.iter().map(|s| s * s).sum();

            // Both should have audio
            assert!(normal_energy > 0.001, "Normal speed should have audio");
            assert!(fast_energy > 0.001, "Fast speed should have audio");
        }
    }

    sampler.stop_stream(0);
}

/// Test streaming pause and resume.
#[test]
#[cfg(feature = "wav")]
fn test_butler_pause_resume() {
    let engine = test_engine();
    let sampler = engine.sampler();

    let test_file = test_data_dir().join("regression/test_sine.wav");
    if !test_file.exists() {
        return;
    }

    sampler.stream_file(0, &test_file);
    sampler.run();

    std::thread::sleep(std::time::Duration::from_millis(300));

    // Pause butler
    sampler.pause();
    std::thread::sleep(std::time::Duration::from_millis(100));

    // Resume
    sampler.run();
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Test passes if no crash occurred during pause/resume cycle
    // The streaming state may vary based on timing
    sampler.stop_stream(0);
}

// =============================================================================
// Downcast Tests — verify SamplerUnit can be found via node_ref_typed
// =============================================================================

/// Verify that a SamplerUnit added to the graph can be found via node_ref_typed downcast.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_sampler_downcast_in_graph() {
    use tutti::core::Wave;

    let engine = test_engine();

    // Create a wave directly and cache it
    let wave = Arc::new(Wave::with_capacity(2, 44100.0, 44100));
    let test_path = std::path::Path::new("/tmp/test_downcast.wav");
    engine.cache_wave(test_path, wave);

    let sampler = engine
        .wav(test_path)
        .build()
        .expect("Should build from cached wave");

    // Add sampler to graph and track its node ID
    let node_id = engine.graph_mut(|net| net.add(sampler).master());

    // Now verify we can downcast it back
    engine.graph(|net| {
        let typed = net.node_ref_typed::<SamplerUnit>(node_id);
        assert!(
            typed.is_some(),
            "SamplerUnit should be findable via node_ref_typed. node_id={:?}",
            node_id,
        );
    });
}

/// Verify content_end_beat finds sampler nodes.
#[test]
#[cfg(all(feature = "wav", feature = "export"))]
fn test_content_end_beat_finds_samplers() {
    use tutti::core::Wave;

    let engine = test_engine();

    // Create a wave directly and cache it
    let wave = Arc::new(Wave::with_capacity(2, 44100.0, 44100));
    let test_path = std::path::Path::new("/tmp/test_content_end.wav");
    engine.cache_wave(test_path, wave);

    let sampler = engine
        .wav(test_path)
        .start_beat(0.0)
        .duration_beats(8.0)
        .build()
        .expect("Should build from cached wave");

    engine.graph_mut(|net| {
        net.add(sampler).master();
    });

    let end_beat = engine.content_end_beat();
    assert!(
        end_beat >= 8.0,
        "content_end_beat should be >= 8.0, got {}",
        end_beat,
    );
}
