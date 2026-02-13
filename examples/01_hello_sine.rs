//! # 01 - Hello Sine
//!
//! Generate a 440Hz sine wave and play it through the default audio device.
//!
//! **Concepts:** Engine setup, audio graph basics, transport
//!
//! ```bash
//! cargo run --example 01_hello_sine
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().build()?;

    engine.graph_mut(|net: &mut TuttiNet| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    engine.transport().play();
    println!("Playing 440Hz sine...");
    std::thread::sleep(Duration::from_secs(3));

    Ok(())
}
