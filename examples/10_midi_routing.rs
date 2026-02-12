//! # 10 - MIDI Synth
//!
//! Create a polyphonic synthesizer that responds to MIDI note_on/note_off.
//!
//! **Concepts:** `engine.synth()`, `note_on`, `note_off`, `Note` enum, polyphony
//!
//! ```bash
//! cargo run --example 10_midi_routing --features "synth,midi"
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().midi().build()?;

    // Create a polyphonic saw synth with Moog filter
    let synth = engine
        .synth()
        .saw()
        .poly(8)
        .filter_moog(2000.0, 0.5)
        .adsr(0.01, 0.1, 0.7, 0.3)
        .build()?;

    let synth_id = engine.graph(|net| net.add(synth).master());

    engine.transport().play();
    println!("Playing arpeggio...");

    // Play a C major arpeggio using Note enum
    let arpeggio = [Note::C4, Note::E4, Note::G4, Note::C5, Note::G4, Note::E4];
    for note in arpeggio {
        engine.note_on(synth_id, note, 100);
        std::thread::sleep(Duration::from_millis(200));
        engine.note_off(synth_id, note);
        std::thread::sleep(Duration::from_millis(50));
    }

    // Play a chord
    println!("Playing chord...");
    let chord = [Note::C4, Note::E4, Note::G4];
    for note in chord {
        engine.note_on(synth_id, note, 100);
    }
    std::thread::sleep(Duration::from_secs(2));
    for note in chord {
        engine.note_off(synth_id, note);
    }
    std::thread::sleep(Duration::from_millis(500));

    Ok(())
}
