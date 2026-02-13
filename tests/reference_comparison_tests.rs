//! Reference file comparison tests (Zrythm-style)
//!
//! Tests that compare rendered output against known-good reference files
//! or programmatically generated reference signals.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test reference_comparison_tests --features "export,wav"
//! ```

#![cfg(all(feature = "export", feature = "wav"))]

#[path = "helpers/mod.rs"]
mod helpers;

use helpers::tolerances::*;
use helpers::{
    assert_signals_equal, compare_audio, generate_sine, load_reference_wav, peak, rms,
    save_reference_wav, save_wav_file, save_wav_file_pcm16, stereo_equal, test_data_dir,
    test_engine,
};
use tutti::prelude::*;

// =============================================================================
// Programmatic Reference Tests
// =============================================================================

/// Test rendered sine wave matches programmatically generated reference.
#[test]
fn test_sine_vs_programmatic_reference() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0)).master();
    });

    let (left, _right, sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Generate reference sine at same frequency
    let reference = generate_sine(440.0, sr, left.len());

    // For sine waves, compare RMS rather than sample-by-sample (phase may differ)
    let rendered_rms = rms(&left);
    let ref_rms = rms(&reference);

    assert!(
        (rendered_rms - ref_rms).abs() < 0.05,
        "RMS should match: rendered={}, reference={}",
        rendered_rms,
        ref_rms
    );

    // Peak should be close to 1.0
    assert!(
        (peak(&left) - 1.0).abs() < DSP_EPSILON,
        "Peak should be ~1.0, got {}",
        peak(&left)
    );
}

/// Test rendered half-amplitude sine matches reference.
#[test]
fn test_scaled_sine_vs_reference() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(1000.0) * 0.5).master();
    });

    let (left, _right, sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Generate reference and scale
    let reference: Vec<f32> = generate_sine(1000.0, sr, left.len())
        .iter()
        .map(|&s| s * 0.5)
        .collect();

    // RMS comparison
    let rendered_rms = rms(&left);
    let ref_rms = rms(&reference);

    assert!(
        (rendered_rms - ref_rms).abs() < 0.05,
        "Scaled RMS should match: rendered={}, reference={}",
        rendered_rms,
        ref_rms
    );
}

/// Test two identical renders produce identical output.
#[test]
fn test_render_determinism() {
    let engine1 = test_engine();
    engine1.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left1, right1, _) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render 1 failed");

    let engine2 = test_engine();
    engine2.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left2, right2, _) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render 2 failed");

    // Should be identical
    assert!(
        stereo_equal(&left1, &right1, &left2, &right2, FLOAT_EPSILON),
        "Two identical renders should produce identical output"
    );
}

// =============================================================================
// File Reference Tests (require reference files to exist)
// =============================================================================

/// Test loading and comparing against a reference file.
/// This test is skipped if the reference file doesn't exist.
#[test]
fn test_load_reference_file() {
    // Skip if reference doesn't exist (normal for first run)
    let ref_path = test_data_dir().join("regression/test_sine.wav");
    if !ref_path.exists() {
        eprintln!("Reference file not found at {:?}, skipping test", ref_path);
        eprintln!("To create: run with GENERATE_REFS=1 or use save_reference_wav()");
        return;
    }

    let (ref_left, ref_right, ref_sr) =
        load_reference_wav("regression/test_sine.wav").expect("Failed to load reference");

    assert!(ref_sr > 0, "Reference should have valid sample rate");
    assert!(!ref_left.is_empty(), "Reference should have samples");
    assert_eq!(
        ref_left.len(),
        ref_right.len(),
        "Stereo channels should match"
    );
}

/// Regression test for filter chain.
/// Marked as ignored - enable when reference file is created.
#[test]
#[ignore]
fn test_filter_chain_regression() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        // Deterministic DSP chain
        let chain = sine_hz::<f64>(440.0) >> lowpole_hz(1000.0) >> highpole_hz(100.0) * 0.5;
        net.add(chain).master();
    });

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    let (ref_left, ref_right, _) = load_reference_wav("regression/filter_chain_v0_1_0.wav")
        .expect("Reference file required for regression test");

    // Compare with DSP tolerance
    let left_result = compare_audio(&left, &ref_left, DSP_EPSILON);
    let right_result = compare_audio(&right, &ref_right, DSP_EPSILON);

    assert!(
        left_result.equal,
        "Left channel differs from reference: max_diff={}, at sample {:?}",
        left_result.max_diff, left_result.first_diff_sample
    );
    assert!(
        right_result.equal,
        "Right channel differs from reference: max_diff={}, at sample {:?}",
        right_result.max_diff, right_result.first_diff_sample
    );
}

// =============================================================================
// Reference Generation Helper (for creating new references)
// =============================================================================

/// Helper test to generate reference files.
/// Run with: cargo test generate_reference_files -- --ignored --nocapture
#[test]
#[ignore]
fn generate_reference_files() {
    use std::fs;

    // Ensure directory exists
    let regression_dir = test_data_dir().join("regression");
    fs::create_dir_all(&regression_dir).expect("Failed to create regression directory");

    // Generate sine reference
    {
        let engine = test_engine();
        engine.graph_mut(|net| {
            net.add(sine_hz::<f64>(440.0) * 0.5).master();
        });

        let (left, right, sr) = engine
            .export()
            .duration_seconds(1.0)
            .render()
            .expect("Render failed");

        save_reference_wav("regression/test_sine.wav", &left, &right, sr as u32)
            .expect("Failed to save reference");
        println!("Generated: regression/test_sine.wav");
    }

    // Generate filter chain reference
    {
        let engine = test_engine();
        engine.graph_mut(|net| {
            let chain = sine_hz::<f64>(440.0) >> lowpole_hz(1000.0) >> highpole_hz(100.0) * 0.5;
            net.add(chain).master();
        });

        let (left, right, sr) = engine
            .export()
            .duration_seconds(0.5)
            .render()
            .expect("Render failed");

        save_reference_wav(
            "regression/filter_chain_v0_1_0.wav",
            &left,
            &right,
            sr as u32,
        )
        .expect("Failed to save reference");
        println!("Generated: regression/filter_chain_v0_1_0.wav");
    }

    println!("Reference files generated in {:?}", regression_dir);
}

// =============================================================================
// Stereo Comparison Tests
// =============================================================================

/// Test that mono source produces identical L/R channels.
#[test]
fn test_mono_to_stereo() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Mono source should produce identical channels
    assert_signals_equal(&left, &right, FLOAT_EPSILON, "Mono L/R channels");
}

/// Test that stereo source produces different L/R when expected.
#[test]
fn test_stereo_difference() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        // Different frequencies for L and R
        let stereo = sine_hz::<f64>(440.0) | sine_hz::<f64>(880.0);
        net.add(stereo * 0.5).master();
    });

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Different frequencies should produce different channels
    let result = compare_audio(&left, &right, DSP_EPSILON);
    assert!(
        !result.equal,
        "Different frequencies should produce different L/R"
    );
}

// =============================================================================
// Round-Trip Quality Tests
// =============================================================================

/// Test WAV save/load round-trip quality.
/// Measures how accurately audio survives the save→load cycle.
#[test]
fn test_wav_round_trip_quality() {
    use std::fs;

    let test_dir = test_data_dir().join("temp");
    fs::create_dir_all(&test_dir).expect("Failed to create temp directory");
    let test_path = test_dir.join("round_trip_test.wav");

    // Generate original signal
    let original = generate_sine(440.0, 48000.0, 4800); // 0.1 seconds
    let original_left = original.clone();
    let original_right = original.clone();

    // Save to WAV
    save_reference_wav(
        "temp/round_trip_test.wav",
        &original_left,
        &original_right,
        48000,
    )
    .expect("Failed to save");

    // Load back
    let (loaded_left, loaded_right, loaded_sr) =
        load_reference_wav("temp/round_trip_test.wav").expect("Failed to load");

    // Cleanup
    let _ = fs::remove_file(&test_path);
    let _ = fs::remove_dir(test_dir);

    // Verify sample rate preserved
    assert_eq!(loaded_sr, 48000, "Sample rate should be preserved");

    // Verify length preserved
    assert_eq!(
        loaded_left.len(),
        original_left.len(),
        "Left channel length should be preserved"
    );
    assert_eq!(
        loaded_right.len(),
        original_right.len(),
        "Right channel length should be preserved"
    );

    // Compare with detailed metrics
    let left_result = compare_audio(&original_left, &loaded_left, FLOAT_EPSILON);
    let right_result = compare_audio(&original_right, &loaded_right, FLOAT_EPSILON);

    // Print quality metrics
    println!("\n=== WAV Round-Trip Quality Report ===");
    println!("Sample rate: {} Hz", loaded_sr);
    println!("Samples: {}", loaded_left.len());
    println!("\nLeft channel:");
    println!("  Max diff: {:.2e}", left_result.max_diff);
    println!("  Mean diff: {:.2e}", left_result.mean_diff);
    println!("  Samples differing: {}", left_result.num_diffs);
    println!("\nRight channel:");
    println!("  Max diff: {:.2e}", right_result.max_diff);
    println!("  Mean diff: {:.2e}", right_result.mean_diff);
    println!("  Samples differing: {}", right_result.num_diffs);

    // WAV uses 32-bit float, should be lossless for f32
    assert!(
        left_result.max_diff < FLOAT_EPSILON,
        "Left channel round-trip error too high: {:.2e} (threshold: {:.2e})",
        left_result.max_diff,
        FLOAT_EPSILON
    );
    assert!(
        right_result.max_diff < FLOAT_EPSILON,
        "Right channel round-trip error too high: {:.2e} (threshold: {:.2e})",
        right_result.max_diff,
        FLOAT_EPSILON
    );

    println!("\nResult: LOSSLESS (within {:.0e})", FLOAT_EPSILON);
}

/// Test engine render → WAV file → load (hound only, not through engine).
#[test]
fn test_engine_render_wav_file_round_trip() {
    use std::fs;

    let test_dir = test_data_dir().join("temp");
    fs::create_dir_all(&test_dir).expect("Failed to create temp directory");

    // Render from engine
    let engine = test_engine();
    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (original_left, original_right, sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Save to WAV
    save_reference_wav(
        "temp/engine_round_trip.wav",
        &original_left,
        &original_right,
        sr as u32,
    )
    .expect("Failed to save");

    // Load back (via hound, not engine)
    let (loaded_left, loaded_right, loaded_sr) =
        load_reference_wav("temp/engine_round_trip.wav").expect("Failed to load");

    // Cleanup
    let _ = fs::remove_file(test_dir.join("engine_round_trip.wav"));
    let _ = fs::remove_dir(test_dir);

    // Verify
    assert_eq!(loaded_sr as f64, sr, "Sample rate should match");

    let left_result = compare_audio(&original_left, &loaded_left, FLOAT_EPSILON);
    let right_result = compare_audio(&original_right, &loaded_right, FLOAT_EPSILON);

    println!("\n=== Engine Render → WAV File Round-Trip (hound) ===");
    println!("Left max diff: {:.2e}", left_result.max_diff);
    println!("Right max diff: {:.2e}", right_result.max_diff);

    assert!(
        left_result.equal,
        "Left channel should round-trip losslessly"
    );
    assert!(
        right_result.equal,
        "Right channel should round-trip losslessly"
    );
}

/// Full engine round-trip: render → save WAV → load into engine → render → compare.
/// This tests the actual sampler/audio loading path.
#[test]
#[cfg(feature = "sampler")]
fn test_full_engine_round_trip() {
    use std::fs;

    let test_dir = test_data_dir().join("temp");
    fs::create_dir_all(&test_dir).expect("Failed to create temp directory");
    let wav_path = test_dir.join("full_round_trip.wav");

    // Step 1: Render original audio from engine
    let engine1 = test_engine();
    engine1.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (original_left, original_right, sr) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    println!("\n=== Full Engine Round-Trip Test ===");
    println!("Original: {} samples at {} Hz", original_left.len(), sr);
    println!("Original peak L: {:.6}", peak(&original_left));
    println!("Original peak R: {:.6}", peak(&original_right));

    // Step 2: Save to WAV file (32-bit float for lossless round-trip)
    save_wav_file(&wav_path, &original_left, &original_right, sr as u32)
        .expect("Failed to save WAV");

    // Step 3: Load WAV into a new engine and render
    let engine2 = TuttiEngine::builder()
        .build()
        .expect("Failed to build engine2");

    // New fluent API: engine.wav(path).build() returns AudioUnit
    let sampler = engine2
        .wav(&wav_path)
        .build()
        .expect("Failed to load WAV into engine");

    engine2.graph_mut(|net| {
        net.add(sampler).master();
    });

    let (loaded_left, loaded_right, loaded_sr): (Vec<f32>, Vec<f32>, f64) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Re-render failed");

    // Cleanup
    let _ = fs::remove_file(&wav_path);
    let _ = fs::remove_dir(&test_dir);

    println!("Loaded: {} samples at {} Hz", loaded_left.len(), loaded_sr);
    println!("Loaded peak L: {:.6}", peak(&loaded_left));
    println!("Loaded peak R: {:.6}", peak(&loaded_right));

    // Step 4: Compare
    // Note: Sample lengths might differ slightly due to buffer alignment
    let compare_len = std::cmp::min(original_left.len(), loaded_left.len());

    let left_result = compare_audio(
        &original_left[..compare_len],
        &loaded_left[..compare_len],
        DSP_EPSILON,
    );
    let right_result = compare_audio(
        &original_right[..compare_len],
        &loaded_right[..compare_len],
        DSP_EPSILON,
    );

    println!("\nComparison ({} samples):", compare_len);
    println!(
        "Left - max_diff: {:.2e}, mean_diff: {:.2e}, diffs: {}",
        left_result.max_diff, left_result.mean_diff, left_result.num_diffs
    );
    println!(
        "Right - max_diff: {:.2e}, mean_diff: {:.2e}, diffs: {}",
        right_result.max_diff, right_result.mean_diff, right_result.num_diffs
    );

    // Allow DSP_EPSILON tolerance for the full round-trip
    // (WAV loading may involve resampling, format conversion, etc.)
    assert!(
        left_result.max_diff < DSP_EPSILON,
        "Left channel round-trip error too high: {:.2e}",
        left_result.max_diff
    );
    assert!(
        right_result.max_diff < DSP_EPSILON,
        "Right channel round-trip error too high: {:.2e}",
        right_result.max_diff
    );

    println!("\nResult: PASS (within DSP tolerance {:.0e})", DSP_EPSILON);
}
