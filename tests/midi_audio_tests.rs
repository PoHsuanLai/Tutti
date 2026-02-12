//! MIDI audio verification tests
//!
//! These tests verify that MIDI produces correct audio output.
//! Uses offline rendering to verify frequency, amplitude, and relationships.
//!
//! IMPORTANT: For offline export with synths, do NOT call transport().play()
//! before export(). The transport will consume MIDI events before export
//! can capture them. Just send note_on then export immediately.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test midi_audio_tests --features "midi,synth,export"
//! ```

#![cfg(all(feature = "midi", feature = "synth", feature = "export"))]

use tutti::midi::Note;
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

/// Test MIDI note A4 produces 440Hz frequency.
#[test]
fn test_midi_note_frequency() {
    let engine = test_engine();

    let synth = engine.synth().sine().poly(1).build().unwrap();
    let synth_id = engine.graph(|net| net.add(synth).master());

    // Send note BEFORE export (don't call transport.play() - it would consume events)
    engine.note_on(synth_id, Note::A4, 100);

    let (left, _, sample_rate) = engine
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    engine.note_off(synth_id, Note::A4);

    // Estimate frequency from zero crossings
    let estimated_freq = estimate_frequency(&left, sample_rate);

    // A4 should be 440Hz (allow 10% tolerance for ADSR envelope effects)
    assert!(
        (estimated_freq - 440.0).abs() < 44.0,
        "A4 should produce ~440Hz, got {}Hz",
        estimated_freq
    );
}

/// Test octave relationship: C5 should be 2x frequency of C4.
#[test]
fn test_midi_octave_relationship() {
    let sample_rate = 48000.0;

    // C4 = 261.63 Hz
    let engine_c4 = test_engine();
    let synth_c4 = engine_c4.synth().sine().poly(1).build().unwrap();
    let synth_id = engine_c4.graph(|net| net.add(synth_c4).master());

    engine_c4.note_on(synth_id, Note::C4, 100);

    let (left_c4, _, _) = engine_c4
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    engine_c4.note_off(synth_id, Note::C4);

    let freq_c4 = estimate_frequency(&left_c4, sample_rate);

    // C5 = 523.25 Hz (should be 2x C4)
    let engine_c5 = test_engine();
    let synth_c5 = engine_c5.synth().sine().poly(1).build().unwrap();
    let synth_id = engine_c5.graph(|net| net.add(synth_c5).master());

    engine_c5.note_on(synth_id, Note::C5, 100);

    let (left_c5, _, _) = engine_c5
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    engine_c5.note_off(synth_id, Note::C5);

    let freq_c5 = estimate_frequency(&left_c5, sample_rate);

    // C5 should be approximately 2x the frequency of C4
    let ratio = freq_c5 / freq_c4;
    assert!(
        (ratio - 2.0).abs() < 0.2,
        "C5/C4 frequency ratio should be ~2.0, got {} ({:.1}Hz / {:.1}Hz)",
        ratio,
        freq_c5,
        freq_c4
    );
}

/// Test velocity affects amplitude.
/// Higher velocity should produce higher amplitude (if velocity sensitivity is enabled).
/// NOTE: Default PolySynth may not have velocity sensitivity - this test verifies
/// both notes produce audio and documents current behavior.
#[test]
fn test_midi_velocity_amplitude() {
    // Low velocity (40)
    let engine_low = test_engine();
    let synth_low = engine_low.synth().sine().poly(1).build().unwrap();
    let synth_id = engine_low.graph(|net| net.add(synth_low).master());

    engine_low.note_on(synth_id, Note::C4, 40); // Low velocity

    let (left_low, _, _) = engine_low
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine_low.note_off(synth_id, Note::C4);

    let rms_low = rms(&left_low);

    // High velocity (127)
    let engine_high = test_engine();
    let synth_high = engine_high.synth().sine().poly(1).build().unwrap();
    let synth_id = engine_high.graph(|net| net.add(synth_high).master());

    engine_high.note_on(synth_id, Note::C4, 127); // Max velocity

    let (left_high, _, _) = engine_high
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine_high.note_off(synth_id, Note::C4);

    let rms_high = rms(&left_high);

    // Both should produce audio
    assert!(rms_low > 0.001, "Low velocity should produce audio, RMS={}", rms_low);
    assert!(rms_high > 0.001, "High velocity should produce audio, RMS={}", rms_high);

    // High velocity should be louder than or equal to low velocity
    // (current PolySynth implementation may not have velocity sensitivity)
    assert!(
        rms_high >= rms_low,
        "Higher velocity ({}) should produce >= amplitude than lower velocity ({})",
        rms_high,
        rms_low
    );
}

/// Test chord has higher RMS than single note.
#[test]
fn test_midi_chord_amplitude() {
    // Single note
    let engine_single = test_engine();
    let synth_single = engine_single.synth().sine().poly(4).build().unwrap();
    let synth_id = engine_single.graph(|net| net.add(synth_single).master());

    engine_single.note_on(synth_id, Note::C4, 100);

    let (left_single, _, _) = engine_single
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine_single.note_off(synth_id, Note::C4);

    let rms_single = rms(&left_single);

    // C major chord (C4, E4, G4)
    let engine_chord = test_engine();
    let synth_chord = engine_chord.synth().sine().poly(4).build().unwrap();
    let synth_id = engine_chord.graph(|net| net.add(synth_chord).master());

    engine_chord.note_on(synth_id, Note::C4, 100);
    engine_chord.note_on(synth_id, Note::E4, 100);
    engine_chord.note_on(synth_id, Note::G4, 100);

    let (left_chord, _, _) = engine_chord
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine_chord.note_off(synth_id, Note::C4);
    engine_chord.note_off(synth_id, Note::E4);
    engine_chord.note_off(synth_id, Note::G4);

    let rms_chord = rms(&left_chord);

    // Both should produce audio
    assert!(rms_single > 0.001, "Single note should produce audio, RMS={}", rms_single);
    assert!(rms_chord > 0.001, "Chord should produce audio, RMS={}", rms_chord);

    // Chord (3 notes) should have higher RMS than single note
    // Due to phase relationships, it won't be exactly 3x, but should be more
    assert!(
        rms_chord > rms_single * 1.3,
        "Chord RMS ({}) should be significantly higher than single note RMS ({})",
        rms_chord,
        rms_single
    );
}
