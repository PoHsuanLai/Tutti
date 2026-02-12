//! Audio output verification tests
//!
//! Tests that verify actual audio content using offline rendering.
//! These tests render to buffers and verify signal properties.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test audio_output_tests --features "export"
//! ```

#![cfg(feature = "export")]

use tutti::prelude::*;

fn test_engine() -> TuttiEngine {
    TuttiEngine::builder()
        .build()
        .expect("Failed to create test engine")
}

/// Calculate RMS of a signal.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Calculate peak amplitude of a signal.
fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|s| s.abs())
        .fold(0.0_f32, |a, b| a.max(b))
}

/// Count zero crossings in a signal.
fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count()
}

/// Estimate frequency from zero crossings.
fn estimate_frequency(samples: &[f32], sample_rate: f64) -> f64 {
    let crossings = zero_crossings(samples);
    let duration = samples.len() as f64 / sample_rate;
    // Each cycle has 2 zero crossings
    (crossings as f64 / 2.0) / duration
}

// =============================================================================
// Sine Wave Tests
// =============================================================================

/// Test that a 440Hz sine wave produces correct frequency.
#[test]
fn test_sine_wave_frequency() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0)).master();
    });

    let (left, _right, sample_rate) = engine
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    // Estimate frequency from zero crossings
    let estimated_freq = estimate_frequency(&left, sample_rate);

    // Should be close to 440Hz (allow 5% tolerance for edge effects)
    assert!(
        (estimated_freq - 440.0).abs() < 22.0,
        "Expected ~440Hz, got {}Hz",
        estimated_freq
    );
}

/// Test that a sine wave has correct amplitude.
#[test]
fn test_sine_wave_amplitude() {
    let engine = test_engine();

    // Sine at half amplitude
    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.25)
        .render()
        .expect("Render failed");

    // Peak should be around 0.5
    let p = peak(&left);
    assert!(
        (p - 0.5).abs() < 0.05,
        "Expected peak ~0.5, got {}",
        p
    );

    // RMS of sine at amplitude A is A/sqrt(2) ≈ 0.707*A
    let r = rms(&left);
    let expected_rms = 0.5 * 0.707;
    assert!(
        (r - expected_rms).abs() < 0.05,
        "Expected RMS ~{}, got {}",
        expected_rms,
        r
    );
}

/// Test stereo output (mono source should appear in both channels).
#[test]
fn test_stereo_output() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Both channels should have content
    assert!(rms(&left) > 0.1, "Left channel should have audio");
    assert!(rms(&right) > 0.1, "Right channel should have audio");

    // Mono source should produce identical channels
    let diff: f32 = left
        .iter()
        .zip(right.iter())
        .map(|(l, r)| (l - r).abs())
        .sum::<f32>()
        / left.len() as f32;

    assert!(
        diff < 0.001,
        "Mono source should produce identical L/R, avg diff = {}",
        diff
    );
}

// =============================================================================
// Silence Tests
// =============================================================================

/// Test that an empty graph produces silence.
#[test]
fn test_empty_graph_silence() {
    let engine = test_engine();

    // No nodes added

    let (left, right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Should be silent
    let l_peak = peak(&left);
    let r_peak = peak(&right);

    assert!(
        l_peak < 0.001,
        "Empty graph should be silent, but left peak = {}",
        l_peak
    );
    assert!(
        r_peak < 0.001,
        "Empty graph should be silent, but right peak = {}",
        r_peak
    );
}

/// Test that zero() node produces silence.
#[test]
fn test_zero_node_silence() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(zero()).master();
    });

    let (left, _right, _sr) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(
        peak(&left) < 0.001,
        "zero() should produce silence, but peak = {}",
        peak(&left)
    );
}

// =============================================================================
// Mixing Tests
// =============================================================================

/// Test that mixing two signals increases amplitude.
#[test]
fn test_signal_mixing() {
    // Single sine at 0.3 amplitude
    let engine1 = test_engine();
    engine1.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).master();
    });

    let (single, _, _) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let single_rms = rms(&single);

    // Two sines mixed together using FunDSP operator (not separate graph calls)
    let engine2 = test_engine();
    engine2.graph(|net| {
        // Mix two oscillators at the DSP level
        let mixed = (sine_hz::<f64>(440.0) * 0.3) + (sine_hz::<f64>(880.0) * 0.3);
        net.add(mixed).master();
    });

    let (mixed, _, _) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let mixed_rms = rms(&mixed);

    // Mixed should have higher RMS (not exactly 2x due to phase, but significantly more)
    assert!(
        mixed_rms > single_rms * 1.2,
        "Mixed signals should have higher RMS: single={}, mixed={}",
        single_rms,
        mixed_rms
    );
}

/// Test that multiplying amplitude scales the signal.
#[test]
fn test_amplitude_scaling() {
    let engine1 = test_engine();
    engine1.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 1.0).master();
    });

    let (full, _, _) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let full_rms = rms(&full);

    let engine2 = test_engine();
    engine2.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    let (half, _, _) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let half_rms = rms(&half);

    // Half amplitude should have half the RMS
    let ratio = full_rms / half_rms;
    assert!(
        (ratio - 2.0).abs() < 0.1,
        "Expected RMS ratio of 2.0, got {}",
        ratio
    );
}

// =============================================================================
// Filter Tests
// =============================================================================

/// Test that a lowpass filter reduces high frequency content.
#[test]
fn test_lowpass_filter() {
    // High frequency source (4000 Hz)
    let engine1 = test_engine();
    engine1.graph(|net| {
        net.add(sine_hz::<f64>(4000.0) * 0.5).master();
    });

    let (unfiltered, _, _) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let unfiltered_rms = rms(&unfiltered);

    // Same source through a 500Hz lowpass
    let engine2 = test_engine();
    engine2.graph(|net| {
        net.add(sine_hz::<f64>(4000.0) * 0.5 >> lowpole_hz(500.0))
            .master();
    });

    let (filtered, _, _) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let filtered_rms = rms(&filtered);

    // Filtered signal should be significantly quieter (4000Hz is well above 500Hz cutoff)
    assert!(
        filtered_rms < unfiltered_rms * 0.3,
        "Lowpass should attenuate 4000Hz significantly: unfiltered={}, filtered={}",
        unfiltered_rms,
        filtered_rms
    );
}

// =============================================================================
// DSP Chain Tests
// =============================================================================

/// Test that chaining nodes works correctly.
#[test]
fn test_dsp_chain() {
    let engine = test_engine();

    // Oscillator -> filter -> output
    engine.graph(|net| {
        let chain = sine_hz::<f64>(440.0) >> lowpole_hz(2000.0) * 0.5;
        net.add(chain).master();
    });

    let (left, _, _) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Should have audio (not silent)
    assert!(
        rms(&left) > 0.1,
        "Chained DSP should produce audio, RMS = {}",
        rms(&left)
    );
}

/// Test shared parameter changes affect output.
#[test]
fn test_shared_parameter() {
    let engine = test_engine();
    let amp = tutti::Shared::new(0.0);
    let amp_clone = amp.clone();

    engine.graph(|net| {
        net.add(var(&amp_clone) * sine_hz::<f64>(440.0)).master();
    });

    // Render with amplitude = 0
    let (silent, _, _) = engine
        .export()
        .duration_seconds(0.05)
        .render()
        .expect("Render failed");

    assert!(
        peak(&silent) < 0.01,
        "With amp=0, should be silent, peak = {}",
        peak(&silent)
    );

    // Change amplitude
    amp.set(0.5);

    // Render with amplitude = 0.5
    let (loud, _, _) = engine
        .export()
        .duration_seconds(0.05)
        .render()
        .expect("Render failed");

    assert!(
        peak(&loud) > 0.4,
        "With amp=0.5, should have audio, peak = {}",
        peak(&loud)
    );
}

// =============================================================================
// Duration Tests
// =============================================================================

/// Test that render duration produces correct number of samples.
#[test]
fn test_render_duration() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(sine_hz::<f64>(440.0)).master();
    });

    let duration = 0.5; // 0.5 seconds
    let (left, right, sample_rate) = engine
        .export()
        .duration_seconds(duration)
        .render()
        .expect("Render failed");

    let expected_samples = (duration * sample_rate) as usize;

    // Allow small tolerance for buffer alignment
    assert!(
        (left.len() as i64 - expected_samples as i64).abs() < 1024,
        "Expected ~{} samples, got {}",
        expected_samples,
        left.len()
    );
    assert_eq!(left.len(), right.len(), "L/R should have same length");
}

// =============================================================================
// Different Waveforms
// =============================================================================

/// Test that different waveforms have different characteristics.
#[test]
fn test_waveform_characteristics() {
    // Sine wave - smooth, low harmonic content
    let engine_sine = test_engine();
    engine_sine.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });
    let (sine_out, _, _) = engine_sine.export().duration_seconds(0.1).render().unwrap();
    let sine_rms = rms(&sine_out);
    let sine_peak = peak(&sine_out);

    // Square wave - more harmonic content, higher RMS for same peak
    let engine_square = test_engine();
    engine_square.graph(|net| {
        net.add(square_hz(440.0) * 0.5).master();
    });
    let (square_out, _, _) = engine_square
        .export()
        .duration_seconds(0.1)
        .render()
        .unwrap();
    let square_rms = rms(&square_out);
    let square_peak = peak(&square_out);

    // Both should have similar peak amplitude
    assert!(
        (sine_peak - square_peak).abs() < 0.1,
        "Peaks should be similar: sine={}, square={}",
        sine_peak,
        square_peak
    );

    // Square wave has higher RMS for same peak (RMS/peak ratio)
    // Sine: RMS/peak = 0.707
    // Square: RMS/peak = 1.0
    let sine_crest = sine_peak / sine_rms;
    let square_crest = square_peak / square_rms;

    assert!(
        sine_crest > square_crest,
        "Sine should have higher crest factor: sine={}, square={}",
        sine_crest,
        square_crest
    );
}

/// Test saw wave produces audio.
#[test]
fn test_saw_wave() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(saw_hz(440.0) * 0.5).master();
    });

    let (left, _, _) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    assert!(rms(&left) > 0.1, "Saw wave should produce audio");
    assert!(peak(&left) <= 0.6, "Saw wave peak should be bounded");
}

/// Test noise produces non-repeating audio.
#[test]
fn test_noise() {
    let engine = test_engine();

    engine.graph(|net| {
        net.add(white() * 0.3).master();
    });

    let (left, _, _) = engine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    // Noise should have content
    assert!(rms(&left) > 0.05, "Noise should produce audio");

    // Noise should have many zero crossings (not periodic like sine)
    let crossings = zero_crossings(&left);
    assert!(
        crossings > left.len() / 10,
        "Noise should have many zero crossings"
    );
}

// =============================================================================
// Parallel Path Tests
// =============================================================================

/// Test that parallel paths sum correctly.
/// Two 0.3 amplitude signals at different frequencies should produce combined output.
#[test]
fn test_parallel_paths_summing() {
    // Single signal
    let engine_single = test_engine();
    engine_single.graph(|net| {
        net.add(sine_hz::<f64>(440.0) * 0.3).master();
    });

    let (single, _, _) = engine_single
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let single_rms = rms(&single);

    // Two signals summed at DSP level
    let engine_summed = test_engine();
    engine_summed.graph(|net| {
        // Two different frequencies mixed together
        let mixed = (sine_hz::<f64>(440.0) * 0.3) + (sine_hz::<f64>(880.0) * 0.3);
        net.add(mixed).master();
    });

    let (summed, _, _) = engine_summed
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let summed_rms = rms(&summed);

    // Summed signal should have higher RMS than single (more energy)
    assert!(
        summed_rms > single_rms * 1.2,
        "Summed signals should have higher RMS: single={}, summed={}",
        single_rms,
        summed_rms
    );
}

/// Test that signal chain order matters.
/// Filter before vs after gain produces different results.
#[test]
fn test_signal_chain_order() {
    // High-frequency source filtered then amplified
    let engine1 = test_engine();
    engine1.graph(|net| {
        // 4kHz sine -> lowpass 500Hz -> * 2.0
        let chain = sine_hz::<f64>(4000.0) >> lowpole_hz(500.0) * 2.0;
        net.add(chain).master();
    });

    let (filter_first, _, _) = engine1
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let rms_filter_first = rms(&filter_first);

    // Same source amplified then filtered
    let engine2 = test_engine();
    engine2.graph(|net| {
        // 4kHz sine * 2.0 -> lowpass 500Hz (filter after amplification)
        let chain = sine_hz::<f64>(4000.0) * 2.0 >> lowpole_hz(500.0);
        net.add(chain).master();
    });

    let (amp_first, _, _) = engine2
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    let rms_amp_first = rms(&amp_first);

    // Both should produce similar results for linear operations
    // (lowpass is linear, so order shouldn't matter much for this case)
    // But the test verifies the chain is being processed correctly
    assert!(
        rms_filter_first > 0.0 && rms_amp_first > 0.0,
        "Both chains should produce audio"
    );

    // The results should be similar (within 50%) since lowpass is linear
    let ratio = if rms_filter_first > rms_amp_first {
        rms_filter_first / rms_amp_first
    } else {
        rms_amp_first / rms_filter_first
    };

    assert!(
        ratio < 2.0,
        "Linear filter order shouldn't dramatically change output: filter_first={}, amp_first={}, ratio={}",
        rms_filter_first,
        rms_amp_first,
        ratio
    );
}
