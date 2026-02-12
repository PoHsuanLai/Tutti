//! Silence detection tests (Zrythm-style)
//!
//! Tests that verify silence detection across various scenarios.
//! Inspired by Zrythm's `audio_file_is_silent` pattern.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test silence_detection_tests --features "export"
//! ```

#![cfg(feature = "export")]

#[path = "helpers/mod.rs"]
mod helpers;

use helpers::tolerances::*;
use helpers::{
    assert_is_silent, assert_not_silent, generate_dc, generate_impulse, generate_silence,
    generate_sine, is_silent, peak, test_engine,
};
use tutti::prelude::*;

// =============================================================================
// Basic Silence Tests
// =============================================================================

/// Test that an empty graph produces true silence.
#[test]
fn test_empty_graph_is_silent() {
    let engine = test_engine();

    // No nodes added

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(
        is_silent(&left, SILENCE_THRESHOLD),
        "Empty graph left channel should be silent, peak = {}",
        peak(&left)
    );
    assert!(
        is_silent(&right, SILENCE_THRESHOLD),
        "Empty graph right channel should be silent, peak = {}",
        peak(&right)
    );
}

/// Test that zero() node produces true silence.
#[test]
fn test_zero_node_is_silent() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(zero()).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(is_silent(&left, SILENCE_THRESHOLD));
}

/// Test that zero gain on signal produces silence.
#[test]
fn test_zero_gain_is_silent() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(white() * 0.0).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(is_silent(&left, SILENCE_THRESHOLD));
}

/// Test that DC signal at zero is silent.
#[test]
fn test_dc_zero_is_silent() {
    let dc = generate_dc(0.0, 10000);
    assert!(is_silent(&dc, SILENCE_THRESHOLD));
}

/// Test programmatically generated silence.
#[test]
fn test_generated_silence_is_silent() {
    let silence = generate_silence(10000);
    assert!(is_silent(&silence, SILENCE_THRESHOLD));
}

// =============================================================================
// Non-Silence Tests
// =============================================================================

/// Test that actual sine audio is NOT silent.
#[test]
fn test_sine_is_not_silent() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(
        !is_silent(&left, SILENCE_THRESHOLD),
        "Sine wave should not be silent"
    );
    assert!(
        peak(&left) > 0.4,
        "Sine at 0.5 amplitude should have peak > 0.4, got {}",
        peak(&left)
    );
}

/// Test that noise is NOT silent.
#[test]
fn test_noise_is_not_silent() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(white() * 0.3).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(!is_silent(&left, SILENCE_THRESHOLD));
}

/// Test that saw wave is NOT silent.
#[test]
fn test_saw_is_not_silent() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(saw_hz(440.0) * 0.5).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(!is_silent(&left, SILENCE_THRESHOLD));
}

/// Test that DC offset is NOT silent.
#[test]
fn test_dc_nonzero_is_not_silent() {
    let dc = generate_dc(0.5, 10000);
    assert!(!is_silent(&dc, SILENCE_THRESHOLD));
}

/// Test that generated sine is NOT silent.
#[test]
fn test_generated_sine_is_not_silent() {
    let sine = generate_sine(440.0, 48000.0, 4800);
    assert!(!is_silent(&sine, SILENCE_THRESHOLD));
}

// =============================================================================
// Threshold Tests
// =============================================================================

/// Test silence detection with various thresholds.
#[test]
fn test_silence_thresholds() {
    // Very quiet signal (0.00001 amplitude)
    let quiet = generate_dc(0.00001, 1000);

    // Should be silent with default threshold
    assert!(is_silent(&quiet, SILENCE_THRESHOLD));

    // Should NOT be silent with stricter threshold
    assert!(!is_silent(&quiet, 0.000001));
}

/// Test that impulse is not silent (has one non-zero sample).
#[test]
fn test_impulse_not_silent() {
    let impulse = generate_impulse(1000, 500);

    // Even one non-zero sample makes it non-silent
    assert!(!is_silent(&impulse, SILENCE_THRESHOLD));
}

/// Test near-silent signal (just above threshold).
#[test]
fn test_near_silent() {
    // Signal just above silence threshold
    let near_silent = generate_dc(SILENCE_THRESHOLD * 2.0, 1000);
    assert!(!is_silent(&near_silent, SILENCE_THRESHOLD));

    // Signal just below silence threshold
    let almost_silent = generate_dc(SILENCE_THRESHOLD / 2.0, 1000);
    assert!(is_silent(&almost_silent, SILENCE_THRESHOLD));
}

// =============================================================================
// Assertion Tests
// =============================================================================

/// Test assert_is_silent with valid silence.
#[test]
fn test_assert_is_silent_valid() {
    let silence = generate_silence(1000);
    assert_is_silent(&silence, SILENCE_THRESHOLD, "Generated silence");
}

/// Test assert_not_silent with valid audio.
#[test]
fn test_assert_not_silent_valid() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert_not_silent(&left, 0.1, "Rendered sine");
}

// =============================================================================
// Edge Cases
// =============================================================================

/// Test empty buffer is silent.
#[test]
fn test_empty_buffer_is_silent() {
    let empty: Vec<f32> = vec![];
    assert!(is_silent(&empty, SILENCE_THRESHOLD));
}

/// Test single sample silence.
#[test]
fn test_single_sample_silence() {
    let single = vec![0.0];
    assert!(is_silent(&single, SILENCE_THRESHOLD));
}

/// Test single sample non-silence.
#[test]
fn test_single_sample_not_silent() {
    let single = vec![0.5];
    assert!(!is_silent(&single, SILENCE_THRESHOLD));
}
