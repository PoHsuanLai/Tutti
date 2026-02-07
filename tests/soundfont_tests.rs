//! SoundFont integration tests
//!
//! Tests loading and playing SoundFont (.sf2) files using the fluent builder API.
//!
//! Run with:
//! ```bash
//! cargo test -p tutti --test soundfont_tests --features "midi,soundfont,export"
//! ```

#![cfg(all(feature = "midi", feature = "soundfont", feature = "export"))]

use std::path::PathBuf;
use tutti::midi::Note;
use tutti::prelude::*;

fn test_engine() -> TuttiEngine {
    TuttiEngine::builder()
        .sample_rate(48000.0)
        .build()
        .expect("Failed to create test engine")
}

fn soundfont_path() -> PathBuf {
    // Path relative to the crate root
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/soundfonts/TimGM6mb.sf2")
}

/// Calculate RMS of a signal.
fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

// =============================================================================
// SoundFont Loading Tests
// =============================================================================

/// Test loading a SoundFont file with the new fluent API.
#[test]
fn test_soundfont_load() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    // Skip if soundfont doesn't exist
    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    // New fluent API: engine.sf2(path).build()
    let result = engine.sf2(&sf_path).build();
    assert!(result.is_ok(), "Should load SoundFont: {:?}", result.err());
}

/// Test SoundFont produces audio.
#[test]
fn test_soundfont_produces_audio() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    // New API: build returns AudioUnit, user adds to graph
    let piano = engine.sf2(&sf_path).preset(0).build().expect("Failed to build piano");
    let piano_id = engine.graph(|net| net.add(piano).to_master());

    // Play a note
    engine.note_on(piano_id, Note::C4, 100);

    let (left, right, _) = engine
        .export()
        .duration_seconds(0.5)
        .render()
        .expect("Render failed");

    engine.note_off(piano_id, Note::C4);

    // Should produce audio
    assert!(
        rms(&left) > 0.001,
        "SoundFont should produce left audio, RMS={}",
        rms(&left)
    );
    assert!(
        rms(&right) > 0.001,
        "SoundFont should produce right audio, RMS={}",
        rms(&right)
    );
}

/// Test SoundFont with different presets.
#[test]
fn test_soundfont_presets() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    // Preset 0 = Piano (using fluent builder with preset)
    let piano = engine.sf2(&sf_path).preset(0).build().expect("Failed to build preset 0");
    let piano_id = engine.graph(|net| net.add(piano).to_master());

    engine.note_on(piano_id, Note::C4, 100);

    let (left, right, _) = engine
        .export()
        .duration_seconds(0.3)
        .render()
        .expect("Render failed");

    engine.note_off(piano_id, Note::C4);

    assert!(
        rms(&left) > 0.001,
        "Piano should produce audio, RMS={}",
        rms(&left)
    );
    assert!(
        rms(&right) > 0.001,
        "Piano should produce audio, RMS={}",
        rms(&right)
    );
}

/// Test SoundFont with different channels.
#[test]
fn test_soundfont_channels() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    // Create SoundFont on channel 0
    let sf = engine.sf2(&sf_path).preset(0).channel(0).build().expect("Failed to build");
    let sf_id = engine.graph(|net| net.add(sf).to_master());

    engine.note_on(sf_id, Note::C4, 100);

    let (left, right, _) = engine
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");

    engine.note_off(sf_id, Note::C4);

    assert!(
        rms(&left) > 0.001,
        "Channel 0 should produce audio, RMS={}",
        rms(&left)
    );
    assert!(
        rms(&right) > 0.001,
        "Channel 0 should produce audio, RMS={}",
        rms(&right)
    );
}

/// Test SoundFont note velocity affects volume.
#[test]
fn test_soundfont_velocity() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    let sf = engine.sf2(&sf_path).preset(0).build().expect("Failed to build");
    let sf_id = engine.graph(|net| net.add(sf).to_master());

    // Soft note
    engine.note_on(sf_id, Note::C4, 30);
    let (left_soft, _, _) = engine
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");
    engine.note_off(sf_id, Note::C4);
    let rms_soft = rms(&left_soft);

    // Loud note
    engine.note_on(sf_id, Note::C4, 127);
    let (left_loud, _, _) = engine
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");
    engine.note_off(sf_id, Note::C4);
    let rms_loud = rms(&left_loud);

    // Loud should be louder than soft
    assert!(
        rms_loud > rms_soft,
        "Loud (vel=127) RMS={} should be > soft (vel=30) RMS={}",
        rms_loud,
        rms_soft
    );
}

/// Test SoundFont polyphony (multiple notes).
#[test]
fn test_soundfont_polyphony() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    let sf = engine.sf2(&sf_path).preset(0).build().expect("Failed to build");
    let sf_id = engine.graph(|net| net.add(sf).to_master());

    // Single note
    engine.note_on(sf_id, Note::C4, 80);
    let (left_single, _, _) = engine
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");
    engine.note_off(sf_id, Note::C4);
    let rms_single = rms(&left_single);

    // Chord (multiple notes)
    engine.note_on(sf_id, Note::C4, 80);
    engine.note_on(sf_id, Note::E4, 80);
    engine.note_on(sf_id, Note::G4, 80);
    let (left_chord, _, _) = engine
        .export()
        .duration_seconds(0.2)
        .render()
        .expect("Render failed");
    engine.note_off(sf_id, Note::C4);
    engine.note_off(sf_id, Note::E4);
    engine.note_off(sf_id, Note::G4);
    let rms_chord = rms(&left_chord);

    // Chord should be louder than single note
    assert!(
        rms_chord > rms_single,
        "Chord RMS={} should be > single note RMS={}",
        rms_chord,
        rms_single
    );
}

/// Test multiple SoundFont instances.
#[test]
fn test_soundfont_multiple_instances() {
    let engine = test_engine();
    let sf_path = soundfont_path();

    if !sf_path.exists() {
        eprintln!("Skipping test: SoundFont not found at {:?}", sf_path);
        return;
    }

    // Create two separate SoundFont instances (same file, cached internally)
    let piano = engine.sf2(&sf_path).preset(0).build().expect("Failed to build piano");
    let strings = engine.sf2(&sf_path).preset(48).build().expect("Failed to build strings");

    let piano_id = engine.graph(|net| net.add(piano).to_master());
    let strings_id = engine.graph(|net| net.add(strings).to_master());

    // Play notes on both
    engine.note_on(piano_id, Note::C4, 100);
    engine.note_on(strings_id, Note::G4, 100);

    let (left, right, _) = engine
        .export()
        .duration_seconds(0.3)
        .render()
        .expect("Render failed");

    engine.note_off(piano_id, Note::C4);
    engine.note_off(strings_id, Note::G4);

    // Both should contribute to audio
    assert!(
        rms(&left) > 0.001,
        "Multiple instances should produce audio, RMS={}",
        rms(&left)
    );
    assert!(
        rms(&right) > 0.001,
        "Multiple instances should produce audio, RMS={}",
        rms(&right)
    );
}
