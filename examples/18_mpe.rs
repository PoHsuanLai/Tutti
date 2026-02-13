//! # 18 - MPE (MIDI Polyphonic Expression)
//!
//! Configure MPE zones and read per-note expression data (pitch bend, pressure, slide).
//!
//! **Concepts:** MpeMode, MpeZoneConfig, per-note expression, channel allocation
//!
//! ```bash
//! cargo run --example 18_mpe --features midi,mpe
//! ```

use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().midi().build()?;
    let midi = engine.midi();
    let mpe = midi.mpe();

    println!("MPE enabled: {}", mpe.is_enabled());
    println!("Has lower zone: {}", mpe.has_lower_zone());
    println!("Has upper zone: {}", mpe.has_upper_zone());

    // Allocate MPE member channels for notes
    if let Some(ch) = mpe.allocate_channel(60) {
        println!("\nAllocated channel {ch} for note C4 (60)");

        // Send note on the allocated member channel
        midi.send().note_on(ch, 60, 100);

        // Send per-note expression on the member channel
        midi.send()
            .pitch_bend(ch, 4096) // Per-note pitch bend
            .cc(ch, 74, 100); // CC74 = Slide

        // Read back expression values via MPE handle
        println!("Per-note pitch bend: {:.3}", mpe.pitch_bend(60));
        println!("Per-note pressure:   {:.3}", mpe.pressure(60));
        println!("Per-note slide:      {:.3}", mpe.slide(60));
        println!("Note active:         {}", mpe.is_note_active(60));

        // Global pitch bend on master channel (ch 0 for lower zone)
        midi.send().pitch_bend(0, -2048);
        println!("\nGlobal pitch bend:   {:.3}", mpe.pitch_bend_global());
        println!("Combined pitch bend: {:.3}", mpe.pitch_bend(60));

        // Release
        midi.send().note_off(ch, 60, 0);
        mpe.release_channel(60);
        println!("\nAfter release — note active: {}", mpe.is_note_active(60));
    } else {
        println!("No MPE channels available (MPE may not be configured)");
    }

    // Allocate multiple notes to show polyphonic channel assignment
    println!("\n--- Polyphonic channel allocation ---");
    let notes = [60, 64, 67]; // C major triad
    for &note in &notes {
        if let Some(ch) = mpe.allocate_channel(note) {
            println!("Note {note} → channel {ch}");
        }
    }

    // Clean up
    mpe.reset();
    println!("\nMPE state reset.");

    Ok(())
}
