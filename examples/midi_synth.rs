//! MIDI-controlled synthesizer with keyboard simulation
//!
//! Demonstrates: Dynamic note triggering, polyphonic synthesis, MIDI note conversion
//!
//! Run with: cargo run --example midi_synth --features=midi

#[cfg(not(feature = "midi"))]
fn main() {
    eprintln!("This example requires the 'midi' feature.");
    eprintln!("Run with: cargo run --example midi_synth --features=midi");
}

#[cfg(feature = "midi")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use tutti::prelude::*;
    use std::collections::HashMap;
    use std::io::{self, Write};

    let engine = TuttiEngine::builder().sample_rate(44100.0).midi().build()?;

    let _midi = engine.midi().expect("MIDI system not available");

    println!("MIDI Polyphonic Synthesizer");
    println!("===========================");
    println!();
    println!("Type MIDI note numbers to play (or 'q' to quit):");
    println!("  60 = C4 (middle C)");
    println!("  61 = C#4");
    println!("  62 = D4");
    println!("  ... etc");
    println!();
    println!("Examples:");
    println!("  '60' - Play C4");
    println!("  '64 67 71' - Play C4 + E4 + B4 chord");
    println!();

    // Track active notes: MIDI note -> NodeId
    let mut active_notes: HashMap<u8, NodeId> = HashMap::new();

    engine.transport().play();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let trimmed = input.trim();
        if trimmed == "q" || trimmed == "quit" {
            break;
        }

        // Parse space-separated MIDI note numbers
        let notes: Vec<u8> = trimmed
            .split_whitespace()
            .filter_map(|s| s.parse::<u8>().ok())
            .filter(|&n| n <= 127)
            .collect();

        if notes.is_empty() {
            println!("Invalid input. Enter MIDI note numbers (0-127) or 'q' to quit.");
            continue;
        }

        // Remove old notes and add new ones
        engine.graph(|net| {
            // Clear previous notes
            for (_note, node_id) in active_notes.drain() {
                if net.contains(node_id) {
                    net.remove(node_id);
                }
            }

            // Add new notes
            for &note in &notes {
                let freq = midi_hz::<f32>(note as f32);
                let osc = net.add(Box::new(sine_hz::<f32>(freq) * 0.3));
                active_notes.insert(note, osc);
                net.pipe_output(osc);
            }

            // If no notes, output silence
            if active_notes.is_empty() {
                let silence = net.add(Box::new(dc(0.0)));
                net.pipe_output(silence);
            }
        });

        // Show what's playing
        let note_names = notes
            .iter()
            .map(|&n| {
                let note_name = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
                let octave = (n / 12) as i32 - 1;
                let name_idx = (n % 12) as usize;
                format!("{}{} ({})", note_name[name_idx], octave, n)
            })
            .collect::<Vec<_>>()
            .join(", ");

        println!("Playing: {}", note_names);
    }

    println!("Goodbye!");
    Ok(())
}
