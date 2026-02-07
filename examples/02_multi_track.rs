//! # 02 - Multi-Track Mixing
//!
//! Mix multiple audio sources with independent volume control.
//!
//! **Concepts:** Multiple nodes, volume control, `mix!` macro
//!
//! ```bash
//! cargo run --example 02_multi_track
//! ```

use std::time::Duration;
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        let bass = net.add(sine_hz::<f64>(110.0) * 0.3).id();
        let melody = net.add(sine_hz::<f64>(440.0) * 0.2).id();
        let harmony = net.add(sine_hz::<f64>(550.0) * 0.15).id();
        let perc = net.add(pink::<f64>() * 0.1).id();

        let mixed = mix!(net, bass, melody, harmony, perc);
        net.pipe_output(mixed);
    });

    engine.transport().play();
    println!("Playing multi-track mix...");
    std::thread::sleep(Duration::from_secs(5));

    Ok(())
}
