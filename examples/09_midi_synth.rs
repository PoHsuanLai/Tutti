//! # 09 - MIDI Synth
//!
//! Basic polyphonic synthesis with multiple oscillators.
//!
//! **Concepts:** Polyphony, chord synthesis, `midi` feature
//!
//! ```bash
//! cargo run --example 09_midi_synth --features midi,synth
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // C major chord: C4, E4, G4
    engine.graph(|net| {
        let c = sine_hz::<f32>(261.63) * 0.2; // C4
        let e = sine_hz::<f32>(329.63) * 0.2; // E4
        let g = sine_hz::<f32>(392.00) * 0.2; // G4

        net.add(c + e + g).master();
    });

    engine.transport().play();
    println!("Playing C major chord...");

    std::thread::sleep(Duration::from_secs(2));

    Ok(())
}
