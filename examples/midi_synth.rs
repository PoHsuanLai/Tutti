//! MIDI Synthesizer Example
//!
//! Demonstrates polyphonic synthesis.

use tutti::prelude::*;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Create a simple chord with sine waves
    engine.graph(|net| {
        use fundsp::prelude::*;

        // C major chord: C-E-G (261.63, 329.63, 392.00 Hz)
        let c = sine_hz::<f32>(261.63) * 0.2;
        let e = sine_hz::<f32>(329.63) * 0.2;
        let g = sine_hz::<f32>(392.00) * 0.2;

        let chord = net.add(Box::new(c + e + g));
        net.pipe_output(chord);
    });

    println!("Playing C major chord for 2 seconds...");
    engine.transport().play();
    std::thread::sleep(Duration::from_secs(2));

    Ok(())
}
