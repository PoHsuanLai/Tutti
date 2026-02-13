//! # 11 - Recording
//!
//! Record audio output to disk using the export system.
//!
//! **Concepts:** Offline rendering, export to file, recording workflow
//!
//! ```bash
//! cargo run --example 11_recording --features sampler,export
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().build()?;

    engine.graph_mut(|net: &mut TuttiNet| {
        let osc1 = sine_hz::<f64>(261.63) * 0.3;
        let osc2 = sine_hz::<f64>(329.63) * 0.2;
        let osc3 = sine_hz::<f64>(392.00) * 0.2;
        net.add((osc1 + osc2 + osc3) >> split::<U2>()).master();
    });

    engine.transport().tempo(120.0);

    // Real-time playback
    engine.transport().play();
    println!("Playing...");
    std::thread::sleep(Duration::from_secs(2));
    engine.transport().stop();

    // Offline render to file
    engine
        .export()
        .duration_seconds(3.0)
        .to_file("/tmp/tutti_recording_demo.wav")?;
    println!("Exported: /tmp/tutti_recording_demo.wav");

    Ok(())
}
