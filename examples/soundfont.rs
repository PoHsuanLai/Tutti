//! SoundFont Example
//!
//! Demonstrates loading and playing SoundFont (.sf2) instruments with MIDI.

use std::thread;
use std::time::Duration;
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== SoundFont Example ===\n");

    let soundfont_path = std::env::var("SOUNDFONT_PATH")
        .unwrap_or_else(|_| "assets/soundfonts/TimGM6mb.sf2".to_string());

    if !std::path::Path::new(&soundfont_path).exists() {
        eprintln!("Error: SoundFont file not found");
        eprintln!("\nRun: cd assets/soundfonts && ./download-timgm6mb.sh");
        return Ok(());
    }

    println!("Loading SoundFont: {}", soundfont_path);

    let engine = TuttiEngine::builder()
        .sample_rate(44100.0)
        .outputs(2)
        .build()?;

    // Load SoundFont
    engine.load_sf2("piano", &soundfont_path)?;
    println!("✓ SoundFont loaded\n");

    // Create synth instance
    let synth = engine.instance("piano", &params! {
        "preset" => 0,  // GM Piano
        "bank" => 0,
    })?;

    // Connect to output
    engine.graph(|net| {
        net.pipe_output(synth);
    });

    engine.transport().play();

    // Play a melody
    println!("Playing melody...\n");
    let melody = [
        (60, 500),  // C4
        (62, 500),  // D4
        (64, 500),  // E4
        (65, 500),  // F4
        (67, 500),  // G4
        (69, 500),  // A4
        (67, 500),  // G4
        (64, 500),  // E4
        (60, 1000), // C4
    ];

    for (note, duration_ms) in melody.iter() {
        println!("  Note: {}", note);

        // Send Note On
        engine.note_on(synth, 0, *note as u8, 100);

        // Hold note
        thread::sleep(Duration::from_millis(*duration_ms - 50));

        // Send Note Off
        engine.note_off(synth, 0, *note as u8);

        thread::sleep(Duration::from_millis(50));
    }

    println!("\nPlaying C Major chord...");

    // Play chord
    for note in [60, 64, 67] {
        engine.note_on(synth, 0, note, 100);
    }
    thread::sleep(Duration::from_secs(2));

    // Release chord
    for note in [60, 64, 67] {
        engine.note_off(synth, 0, note);
    }
    thread::sleep(Duration::from_millis(500));

    println!("\n✓ Example complete");

    Ok(())
}
