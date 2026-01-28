//! Effect chain example: Process audio through multiple effects
//!
//! Demonstrates: FunDSP effect nodes, audio graph routing
//!
//! Run with: cargo run --example effect_chain

use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        // Create a sawtooth wave at 110Hz (A2)
        let saw = net.add(Box::new(saw_hz(110.0) * 0.3));

        // Add a lowpass filter at 800Hz
        let filtered = net.add(Box::new(lowpole_hz::<f64>(800.0)));
        net.pipe(saw, filtered);

        // Convert mono to stereo for the reverb
        let stereo = net.add_split();
        net.pipe(filtered, stereo);

        // Add reverb (simple FDN reverb)
        let reverb = net.add(Box::new(reverb_stereo(10.0, 2.0, 0.5)));
        net.pipe_all(stereo, reverb);

        // Output
        net.pipe_output(reverb);
    });

    engine.transport().play();

    println!("Playing sawtooth -> lowpass filter -> reverb");
    println!("Press Ctrl+C to exit.");

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
