//! Live coding example: Dynamically update the audio graph while playing
//!
//! Demonstrates: Real-time graph updates, crossfading between graphs
//!
//! Run with: cargo run --example live_coding

use std::io::{self, Write};
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Start with a sine wave
    engine.graph(|net| {
        let sine = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
        net.pipe_output(sine);
    });

    engine.transport().play();

    println!("Live coding demo - press number to change sound:");
    println!("1: 440Hz sine wave");
    println!("2: 220Hz sawtooth");
    println!("3: 330Hz square wave");
    println!("4: Pink noise");
    println!("5: Chord (C major)");
    println!("q: quit");

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "1" => {
                println!("Switching to sine wave...");
                engine.graph(|net| {
                    let sine = net.add(Box::new(sine_hz::<f64>(440.0) * 0.5));
                    net.pipe_output(sine);
                });
            }
            "2" => {
                println!("Switching to sawtooth...");
                engine.graph(|net| {
                    let saw = net.add(Box::new(saw_hz(220.0) * 0.3));
                    net.pipe_output(saw);
                });
            }
            "3" => {
                println!("Switching to square wave...");
                engine.graph(|net| {
                    let square = net.add(Box::new(square_hz(330.0) * 0.3));
                    net.pipe_output(square);
                });
            }
            "4" => {
                println!("Switching to pink noise...");
                engine.graph(|net| {
                    let noise = net.add(Box::new(pink::<f64>() * 0.2));
                    net.pipe_output(noise);
                });
            }
            "5" => {
                println!("Switching to C major chord...");
                engine.graph(|net| {
                    // C major: C4 (261.63), E4 (329.63), G4 (392.00)
                    let c = sine_hz::<f64>(261.63) * 0.2;
                    let e = sine_hz::<f64>(329.63) * 0.2;
                    let g = sine_hz::<f64>(392.00) * 0.2;
                    let chord = net.add(Box::new(c + e + g));
                    net.pipe_output(chord);
                });
            }
            "q" => {
                println!("Exiting...");
                break;
            }
            _ => {
                println!("Unknown command. Try 1-5 or q to quit.");
            }
        }
    }

    Ok(())
}
