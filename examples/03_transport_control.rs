//! # 03 - Transport Control
//!
//! Interactive transport control: play, stop, tempo, looping, metronome.
//!
//! **Concepts:** Transport state, tempo, loop range, metronome, seeking
//!
//! ```bash
//! cargo run --example 03_transport_control
//! ```

use std::io::{self, Write};
use tutti::prelude::*;
use tutti::Shared;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;

    // Shared frequency that we'll update based on beat position
    // This creates an ascending arpeggio that resets on loop
    let freq = Shared::new(220.0);
    let freq_var = freq.clone();

    // Sine wave with variable frequency
    engine.graph(|net| {
        net.add(var(&freq_var) >> sine::<f32>() * 0.15).to_master();
    });

    // Configure transport - 4 beat loop
    engine
        .transport()
        .tempo(120.0)
        .loop_range(0.0, 4.0)
        .enable_loop();

    // Enable metronome
    engine
        .transport()
        .metronome()
        .volume(0.8)
        .accent_every(4)
        .always();

    println!("Transport Control:");
    println!("  p=play  s=stop  l=toggle loop  m=toggle metronome");
    println!("  +=tempo up  -=tempo down  0=seek to start  q=quit");
    println!();
    println!("Listen: pitch rises with beat (C3→E3→G3→B3), resets on loop!");
    println!();

    // Note frequencies for C major arpeggio (C3, E3, G3, B3)
    let notes = [130.81, 164.81, 196.00, 246.94];

    loop {
        let t = engine.transport();

        // Update frequency based on current beat (creates arpeggio)
        let beat = t.current_beat();
        let beat_index = (beat.floor() as usize) % 4;
        freq.set(notes[beat_index]);

        print!(
            "\r[beat:{:5.2} | {} BPM | loop:{} | metro:{}] > ",
            beat,
            t.get_tempo(),
            if t.is_loop_enabled() { "on" } else { "off" },
            if t.metronome().get_mode() != tutti::MetronomeMode::Off {
                "on"
            } else {
                "off"
            }
        );
        io::stdout().flush()?;

        // Non-blocking input check with timeout
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "p" => {
                engine.transport().play();
            }
            "s" => {
                engine.transport().stop();
            }
            "l" => {
                let t = engine.transport();
                if t.is_loop_enabled() {
                    t.disable_loop();
                } else {
                    t.enable_loop();
                }
            }
            "m" => {
                let m = engine.transport().metronome();
                if m.get_mode() == tutti::MetronomeMode::Off {
                    m.always();
                } else {
                    m.off();
                }
            }
            "+" => {
                let current = engine.transport().get_tempo();
                engine.transport().tempo(current + 10.0);
            }
            "-" => {
                let current = engine.transport().get_tempo();
                engine.transport().tempo((current - 10.0).max(30.0));
            }
            "0" => {
                engine.transport().seek(0.0);
            }
            "q" => break,
            _ => {}
        }
    }

    Ok(())
}
