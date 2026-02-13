//! # 06 - Live Coding
//!
//! Dynamically update the audio graph while playing.
//!
//! **Concepts:** Real-time graph updates, hot-swapping DSP
//!
//! ```bash
//! cargo run --example 06_live_coding
//! ```

use std::io::{self, Write};
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().build()?;

    engine.graph_mut(|net: &mut TuttiNet| {
        net.add(sine_hz::<f64>(440.0) * 0.5).master();
    });

    engine.transport().play();
    println!("1=sine 2=saw 3=square 4=noise 5=chord q=quit");

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => engine.graph_mut(|net: &mut TuttiNet| {
                net.add(sine_hz::<f64>(440.0) * 0.5).master();
            }),
            "2" => engine.graph_mut(|net: &mut TuttiNet| {
                net.add(saw_hz(220.0) * 0.3).master();
            }),
            "3" => engine.graph_mut(|net: &mut TuttiNet| {
                net.add(square_hz(330.0) * 0.3).master();
            }),
            "4" => engine.graph_mut(|net: &mut TuttiNet| {
                net.add(pink::<f64>() * 0.2).master();
            }),
            "5" => engine.graph_mut(|net: &mut TuttiNet| {
                let c = sine_hz::<f64>(261.63) * 0.2;
                let e = sine_hz::<f64>(329.63) * 0.2;
                let g = sine_hz::<f64>(392.00) * 0.2;
                net.add(c + e + g).master();
            }),
            "q" => break,
            _ => println!("?"),
        }
    }

    Ok(())
}
