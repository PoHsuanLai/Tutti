//! Multi-track mixer example: Mix multiple audio sources with independent volume control
//!
//! Demonstrates: Multiple audio nodes, volume control, mixing
//!
//! Run with: cargo run --example multi_track

use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        // Track 1: Bass (sine wave at 110Hz)
        let bass = sine_hz::<f64>(110.0) * 0.3;

        // Track 2: Melody (sine wave at 440Hz)
        let melody = sine_hz::<f64>(440.0) * 0.2;

        // Track 3: Harmony (sine wave at 550Hz)
        let harmony = sine_hz::<f64>(550.0) * 0.15;

        // Track 4: Percussion (pink noise burst)
        let perc = pink::<f64>() * 0.1;

        // Mix all tracks together
        let mixed = net.add(Box::new(bass + melody + harmony + perc));
        net.pipe_output(mixed);
    });

    engine.transport().play();

    println!("Playing multi-track mix:");
    println!("  Track 1: Bass (110Hz)");
    println!("  Track 2: Melody (440Hz)");
    println!("  Track 3: Harmony (550Hz)");
    println!("  Track 4: Percussion (pink noise)");
    println!("Press Ctrl+C to exit.");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
