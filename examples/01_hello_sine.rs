//! Basic example: Generate a 440Hz sine wave and play it through the default audio device.
//!
//! Run with: cargo run --example simple_sine

use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create the audio engine with default settings
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Build a simple audio graph with a 440Hz sine wave
    engine.graph(|net| {
        let sine = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
        net.pipe_output(sine);
    });

    // Start playback
    engine.transport().play();

    println!("Playing 440Hz sine wave. Press Ctrl+C to exit.");

    // Keep the program running
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
