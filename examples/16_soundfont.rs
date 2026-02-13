//! # 11 - SoundFont
//!
//! Load and play SoundFont (.sf2) instruments with MIDI.
//!
//! **Concepts:** `engine.sf2()`, `note_on`, `note_off`, SoundFont presets
//!
//! ```bash
//! cargo run --example 11_soundfont --features soundfont
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let soundfont_path = std::env::var("SOUNDFONT_PATH")
        .unwrap_or_else(|_| "assets/soundfonts/TimGM6mb.sf2".to_string());

    if !std::path::Path::new(&soundfont_path).exists() {
        println!("SoundFont not found: {}", soundfont_path);
        println!("Set SOUNDFONT_PATH or run: cd assets/soundfonts && ./download-timgm6mb.sh");
        return Ok(());
    }

    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    // New fluent API: engine.sf2(path).preset(n).build() returns SoundFontUnit
    let piano = engine.sf2(&soundfont_path).preset(0).build()?;
    let synth = engine.graph(|net| net.add(piano).master());

    engine.transport().play();
    println!("Playing melody...");

    // Melody using Note enum
    let melody = [
        (Note::C4, 500),
        (Note::D4, 500),
        (Note::E4, 500),
        (Note::F4, 500),
        (Note::G4, 500),
        (Note::A4, 500),
        (Note::G4, 500),
        (Note::E4, 500),
        (Note::C4, 1000),
    ];

    for (note, duration_ms) in melody {
        engine.note_on(synth, note, 100);
        std::thread::sleep(Duration::from_millis(duration_ms - 50));
        engine.note_off(synth, note);
        std::thread::sleep(Duration::from_millis(50));
    }

    // Play chord
    let chord = [Note::C4, Note::E4, Note::G4];
    for note in chord {
        engine.note_on(synth, note, 100);
    }
    std::thread::sleep(Duration::from_secs(2));
    for note in chord {
        engine.note_off(synth, note);
    }
    std::thread::sleep(Duration::from_millis(500));

    Ok(())
}
