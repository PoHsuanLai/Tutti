//! # 04 - Sequencer
//!
//! Generate rhythmic patterns with beat-based triggering.
//!
//! **Concepts:** Time-based control, rhythmic patterns, graph updates
//!
//! ```bash
//! cargo run --example 04_sequencer
//! ```

use std::time::{Duration, Instant};
use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    engine.graph(|net| {
        net.add(dc(0.0)).master();
    });

    engine.transport().play();
    println!("Playing drum pattern...");

    let pattern = [('K', 0.0), ('H', 0.25), ('S', 0.5), ('H', 0.75)];
    let start = Instant::now();
    let bpm = 120.0;
    let beat_duration = 60.0 / bpm;
    let mut idx = 0;
    let mut last = -1.0_f64;

    while start.elapsed().as_secs_f64() < 4.0 {
        let beat = (start.elapsed().as_secs_f64() / beat_duration) % 1.0;
        let (sound, time) = pattern[idx];

        if beat >= time && last != time {
            last = time;
            print!("{} ", sound);
            match sound {
                'K' => engine.graph(|net| {
                    net.add(sine_hz::<f32>(110.0) * 0.5).master();
                }),
                'S' => engine.graph(|net| {
                    net.add(sine_hz::<f32>(200.0) * 0.4).master();
                }),
                'H' => engine.graph(|net| {
                    net.add(pink::<f64>() >> (bandpass_hz::<f32>(8000.0, 100.0) * 0.2))
                        .master();
                }),
                _ => {}
            }
            idx = (idx + 1) % pattern.len();
            if idx == 0 {
                println!();
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    Ok(())
}
