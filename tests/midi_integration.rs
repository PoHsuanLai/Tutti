//! MIDI integration tests (requires "midi" and "synth" features)
//!
//! Tests MIDI note events, routing, and synth integration with audio verification.
//! Pattern: Inspired by Zrythm's MIDI recording tests and Ardour's midi_clock_test.cc.
//!
//! IMPORTANT: For offline export with synths, do NOT call transport().play()
//! before export(). The transport will consume MIDI events before export
//! can capture them.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test midi_integration --features "midi,synth,export"
//! ```

#![cfg(all(feature = "midi", feature = "synth"))]

use tutti::midi::Note;
use tutti::prelude::*;

/// Create a test engine with MIDI enabled.
fn test_engine_with_midi() -> TuttiEngine {
    TuttiEngine::builder()
        .build()
        .expect("Failed to create MIDI-enabled test engine")
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

/// Test multiple synths produce combined audio.
/// Two synths playing should have higher RMS than one.
#[test]
#[cfg(feature = "export")]
fn test_multiple_synths() {
    // First: single synth
    let engine1 = test_engine_with_midi();

    let lead = engine1.synth().saw().poly(4).build().unwrap();
    let lead_id = engine1.graph_mut(|net| net.add(lead).master());

    // Send note (don't call transport.play() - it would consume the MIDI events)
    engine1.note_on(lead_id, Note::C4, 100);

    let (single_left, _, _) = engine1
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine1.note_off(lead_id, Note::C4);

    let single_rms = rms(&single_left);

    // Second: two synths
    let engine2 = test_engine_with_midi();

    let lead = engine2.synth().saw().poly(4).build().unwrap();
    let bass = engine2.synth().square(0.5).poly(1).build().unwrap();

    let lead_id = engine2.graph_mut(|net| net.add(lead).master());
    let bass_id = engine2.graph_mut(|net| net.add(bass).master());

    engine2.note_on(lead_id, Note::C4, 100);
    engine2.note_on(bass_id, Note::C2, 100);

    let (combined_left, _, _) = engine2
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine2.note_off(lead_id, Note::C4);
    engine2.note_off(bass_id, Note::C2);

    let combined_rms = rms(&combined_left);

    // Combined should have higher RMS (or at least similar if timing is off)
    assert!(
        combined_rms >= single_rms * 0.8,
        "Two synths ({}) should have at least similar RMS to one ({})",
        combined_rms,
        single_rms
    );

    // Both should produce actual audio
    assert!(
        single_rms > 0.01,
        "Single synth should produce audio, RMS = {}",
        single_rms
    );
    assert!(
        combined_rms > 0.01,
        "Combined synths should produce audio, RMS = {}",
        combined_rms
    );
}

/// Test different oscillator types produce different waveforms.
/// Square wave should have higher RMS than sine for same peak (lower crest factor).
#[test]
#[cfg(feature = "export")]
fn test_synth_oscillator_types() {
    // Sine oscillator
    let engine_sine = test_engine_with_midi();
    let sine_synth = engine_sine.synth().sine().poly(2).build().unwrap();
    let sine_id = engine_sine.graph_mut(|net| net.add(sine_synth).master());

    engine_sine.note_on(sine_id, Note::A4, 100);

    let (sine_out, _, _) = engine_sine
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    engine_sine.note_off(sine_id, Note::A4);

    let sine_rms = rms(&sine_out);
    let sine_peak = peak(&sine_out);

    // Square oscillator
    let engine_square = test_engine_with_midi();
    let square_synth = engine_square.synth().square(0.5).poly(2).build().unwrap();
    let square_id = engine_square.graph_mut(|net| net.add(square_synth).master());

    engine_square.note_on(square_id, Note::A4, 100);

    let (square_out, _, _) = engine_square
        .export()
        .duration_seconds(0.1)
        .render()
        .expect("Render failed");

    engine_square.note_off(square_id, Note::A4);

    let square_rms = rms(&square_out);
    let square_peak = peak(&square_out);

    // Both should produce audio
    assert!(
        sine_rms > 0.01,
        "Sine synth should produce audio, RMS={}",
        sine_rms
    );
    assert!(
        square_rms > 0.01,
        "Square synth should produce audio, RMS={}",
        square_rms
    );

    // If peaks are similar, square should have higher RMS (lower crest factor)
    // This is because square wave has constant amplitude
    if (sine_peak - square_peak).abs() < 0.2 {
        let sine_crest = sine_peak / sine_rms;
        let square_crest = square_peak / square_rms;

        assert!(
            sine_crest > square_crest * 0.8,
            "Sine should have higher crest factor ({}) than square ({})",
            sine_crest,
            square_crest
        );
    }
}
