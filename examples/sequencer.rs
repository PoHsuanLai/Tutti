//! Pattern sequencer example: Generate rhythmic patterns programmatically
//!
//! Demonstrates: Time-based control, rhythmic patterns, note triggering
//!
//! Run with: cargo run --example sequencer

use std::time::{Duration, Instant};
use tutti::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Pattern: 4/4 beat with kick, snare, hi-hat
    // K = Kick (110Hz), S = Snare (200Hz), H = Hi-hat (8000Hz noise)
    let pattern = [
        (0.0, 'K'), // Beat 1
        (0.25, 'H'),
        (0.5, 'S'), // Beat 2
        (0.75, 'H'),
        (1.0, 'K'), // Beat 3
        (1.25, 'H'),
        (1.5, 'S'), // Beat 4
        (1.75, 'H'),
    ];

    engine.graph(|net| {
        // Start with silence
        let silence = net.add(Box::new(dc(0.0)));
        net.pipe_output(silence);
    });

    engine.transport().play();

    println!("Pattern sequencer running:");
    println!("K = Kick, S = Snare, H = Hi-hat");
    println!("Pattern: K-H-S-H-K-H-S-H (4/4 beat)");
    println!("Press Ctrl+C to exit.");

    let start = Instant::now();
    let bpm = 120.0;
    let beat_duration = 60.0 / bpm;
    let mut pattern_index = 0;

    loop {
        let elapsed = start.elapsed().as_secs_f64();
        let current_beat = (elapsed / beat_duration) % 2.0; // 2 bars

        if let Some((beat_time, sound)) = pattern.get(pattern_index) {
            if current_beat >= *beat_time && current_beat < *beat_time + 0.1 {
                // Trigger the sound
                match sound {
                    'K' => {
                        println!("Kick");
                        engine.graph(|net| {
                            let kick = net.add(Box::new(sine_hz::<f32>(110.0) * 0.5));
                            net.pipe_output(kick);
                        });
                    }
                    'S' => {
                        println!("Snare");
                        engine.graph(|net| {
                            let snare = net.add(Box::new(sine_hz::<f32>(200.0) * 0.4));
                            net.pipe_output(snare);
                        });
                    }
                    'H' => {
                        println!("Hi-hat");
                        engine.graph(|net| {
                            let hihat = net.add(Box::new(
                                pink::<f64>() >> (bandpass_hz::<f32>(8000.0, 100.0) * 0.2),
                            ));
                            net.pipe_output(hihat);
                        });
                    }
                    _ => {}
                }

                pattern_index = (pattern_index + 1) % pattern.len();
            }
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}
