//! Deterministic signal tests (Ardour-style)
//!
//! Tests that verify exact sample values through passthrough and simple operations.
//! Inspired by Ardour's staircase signal testing pattern.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test deterministic_signal_tests --features "export"
//! ```

#![cfg(feature = "export")]

#[path = "helpers/mod.rs"]
mod helpers;

use helpers::tolerances::*;
use helpers::{
    assert_is_silent, check_staircase, compare_audio, generate_dc, generate_impulse,
    generate_integer_staircase, generate_ramp, generate_sine, is_silent, peak, rms, test_engine,
};
use tutti::prelude::*;

// =============================================================================
// Staircase Signal Tests (Ardour pattern)
// =============================================================================

/// Test that an integer staircase signal renders correctly.
/// This verifies the basic rendering pipeline doesn't corrupt sample values.
#[test]
fn test_staircase_render() {
    // Generate reference staircase
    let reference = generate_integer_staircase(1024);

    // Verify the reference itself is valid
    assert_eq!(reference.len(), 1024);
    assert_eq!(reference[0], 0.0);
    assert_eq!(reference[100], 100.0);
    assert_eq!(reference[1023], 1023.0);

    // Check staircase validation works
    assert!(check_staircase(&reference, 1024, FLOAT_EPSILON));
}

/// Test that rendering produces consistent sample counts.
#[test]
fn test_render_sample_count() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0)).master();
    });

    // 0.1 seconds at 48kHz = 4800 samples
    let (left, right, sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let expected_samples = (0.1 * sr) as usize;

    // Allow small tolerance for buffer alignment
    assert!(
        (left.len() as i64 - expected_samples as i64).abs() < 256,
        "Expected ~{} samples, got {}",
        expected_samples,
        left.len()
    );
    assert_eq!(left.len(), right.len(), "L/R should have same length");
}

// =============================================================================
// Impulse Tests
// =============================================================================

/// Test impulse signal generation.
#[test]
fn test_impulse_generation() {
    let impulse = generate_impulse(512, 0);

    assert_eq!(impulse.len(), 512);
    assert_eq!(impulse[0], 1.0);

    // Rest should be silent
    assert!(is_silent(&impulse[1..], SILENCE_THRESHOLD));
}

/// Test impulse at different positions.
#[test]
fn test_impulse_positions() {
    // Impulse at start
    let imp_start = generate_impulse(100, 0);
    assert_eq!(imp_start[0], 1.0);
    assert!(is_silent(&imp_start[1..], SILENCE_THRESHOLD));

    // Impulse in middle
    let imp_mid = generate_impulse(100, 50);
    assert!(is_silent(&imp_mid[..50], SILENCE_THRESHOLD));
    assert_eq!(imp_mid[50], 1.0);
    assert!(is_silent(&imp_mid[51..], SILENCE_THRESHOLD));

    // Impulse at end
    let imp_end = generate_impulse(100, 99);
    assert!(is_silent(&imp_end[..99], SILENCE_THRESHOLD));
    assert_eq!(imp_end[99], 1.0);
}

// =============================================================================
// DC Offset Tests
// =============================================================================

/// Test DC signal generation.
#[test]
fn test_dc_generation() {
    let dc = generate_dc(0.5, 1000);

    assert_eq!(dc.len(), 1000);
    assert!(dc.iter().all(|&s| (s - 0.5).abs() < FLOAT_EPSILON));
}

/// Test DC at various levels.
#[test]
fn test_dc_levels() {
    // Zero DC
    let zero = generate_dc(0.0, 100);
    assert!(is_silent(&zero, SILENCE_THRESHOLD));

    // Positive DC
    let pos = generate_dc(0.75, 100);
    assert!((rms(&pos) - 0.75).abs() < FLOAT_EPSILON);

    // Negative DC
    let neg = generate_dc(-0.5, 100);
    assert!((rms(&neg) - 0.5).abs() < FLOAT_EPSILON);
}

// =============================================================================
// Ramp Tests
// =============================================================================

/// Test ramp signal generation.
#[test]
fn test_ramp_generation() {
    let ramp = generate_ramp(0.0, 1.0, 101);

    assert_eq!(ramp.len(), 101);
    assert!((ramp[0] - 0.0).abs() < FLOAT_EPSILON);
    assert!((ramp[50] - 0.5).abs() < FLOAT_EPSILON);
    assert!((ramp[100] - 1.0).abs() < FLOAT_EPSILON);
}

/// Test ramp with negative values.
#[test]
fn test_ramp_negative() {
    let ramp = generate_ramp(-1.0, 1.0, 201);

    assert!((ramp[0] - (-1.0)).abs() < FLOAT_EPSILON);
    assert!((ramp[100] - 0.0).abs() < FLOAT_EPSILON);
    assert!((ramp[200] - 1.0).abs() < FLOAT_EPSILON);
}

/// Test descending ramp.
#[test]
fn test_ramp_descending() {
    let ramp = generate_ramp(1.0, 0.0, 11);

    assert!((ramp[0] - 1.0).abs() < FLOAT_EPSILON);
    assert!((ramp[5] - 0.5).abs() < FLOAT_EPSILON);
    assert!((ramp[10] - 0.0).abs() < FLOAT_EPSILON);
}

// =============================================================================
// Gain Tests
// =============================================================================

/// Test that unity gain preserves signal amplitude.
#[test]
fn test_unity_gain_amplitude() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 1.0).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let p = peak(&left);
    assert!(
        (p - 1.0).abs() < DSP_EPSILON,
        "Unity gain should preserve amplitude, peak = {}",
        p
    );
}

/// Test half gain (-6dB) scales correctly.
#[test]
fn test_half_gain_amplitude() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let p = peak(&left);
    assert!(
        (p - 0.5).abs() < DSP_EPSILON,
        "Half gain should produce 0.5 amplitude, peak = {}",
        p
    );
}

/// Test zero gain produces silence.
#[test]
fn test_zero_gain_silence() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.0).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert_is_silent(&left, SILENCE_THRESHOLD, "Zero gain output");
}

/// Test double gain (clipping expected).
#[test]
fn test_double_gain() {
    let engine = test_engine();

    engine.graph_mut(|net| {
        net.add(sine_hz::<f64>(440.0) * 2.0).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Peak should be ~2.0 (no built-in limiter)
    let p = peak(&left);
    assert!(p > 1.5, "Double gain should produce peak > 1.5, got {}", p);
}

// =============================================================================
// Audio Comparison Tests
// =============================================================================

/// Test the compare_audio function with identical signals.
#[test]
fn test_compare_identical() {
    let a = generate_sine(440.0, 48000.0, 4800);
    let b = a.clone();

    let result = compare_audio(&a, &b, FLOAT_EPSILON);

    assert!(result.equal);
    assert_eq!(result.max_diff, 0.0);
    assert_eq!(result.mean_diff, 0.0);
    assert!(result.first_diff_sample.is_none());
    assert_eq!(result.num_diffs, 0);
}

/// Test compare_audio with different signals.
#[test]
fn test_compare_different() {
    let a = generate_sine(440.0, 48000.0, 4800);
    let b = generate_sine(880.0, 48000.0, 4800);

    let result = compare_audio(&a, &b, FLOAT_EPSILON);

    assert!(!result.equal);
    assert!(result.max_diff > 0.1);
    assert!(result.first_diff_sample.is_some());
    assert!(result.num_diffs > 0);
}

/// Test compare_audio with length mismatch.
#[test]
fn test_compare_length_mismatch() {
    let a = generate_sine(440.0, 48000.0, 1000);
    let b = generate_sine(440.0, 48000.0, 2000);

    let result = compare_audio(&a, &b, FLOAT_EPSILON);

    assert!(!result.equal);
    assert_eq!(result.first_diff_sample, Some(0));
}

/// Test compare_audio with small differences within tolerance.
#[test]
fn test_compare_within_tolerance() {
    let a = vec![0.0, 0.5, 1.0];
    let b = vec![0.0001, 0.5001, 0.9999];

    let result = compare_audio(&a, &b, 0.001);

    assert!(result.equal);
    assert!(result.max_diff < 0.001);
}
