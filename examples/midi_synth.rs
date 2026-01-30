//! MIDI-controlled polyphonic synthesizer with presets and sequences
//!
//! Demonstrates: PolySynth, MIDI routing, presets, arpeggios, chord progressions
//!
//! Run with: cargo run --example midi_synth

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{self, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tutti::prelude::*;
    use tutti::synth::{Envelope as SynthEnvelope, PolySynth, Waveform};
    use tutti::AudioUnit;
    use tutti::MidiEvent; // Import trait to bring as_any_mut() into scope

    // Synth presets with different timbres
    #[derive(Clone, Copy)]
    struct SynthPreset {
        name: &'static str,
        waveform: Waveform,
        envelope: SynthEnvelope,
    }

    let presets = [
        SynthPreset {
            name: "Pluck",
            waveform: Waveform::Triangle,
            envelope: SynthEnvelope {
                attack: 0.001,
                decay: 0.3,
                sustain: 0.0,
                release: 0.1,
            },
        },
        SynthPreset {
            name: "Pad",
            waveform: Waveform::Saw,
            envelope: SynthEnvelope {
                attack: 0.8,
                decay: 0.3,
                sustain: 0.6,
                release: 1.2,
            },
        },
        SynthPreset {
            name: "Bass",
            waveform: Waveform::Square,
            envelope: SynthEnvelope {
                attack: 0.01,
                decay: 0.1,
                sustain: 0.8,
                release: 0.2,
            },
        },
        SynthPreset {
            name: "Lead",
            waveform: Waveform::Saw,
            envelope: SynthEnvelope {
                attack: 0.01,
                decay: 0.2,
                sustain: 0.7,
                release: 0.3,
            },
        },
        SynthPreset {
            name: "Bell",
            waveform: Waveform::Sine,
            envelope: SynthEnvelope {
                attack: 0.001,
                decay: 0.8,
                sustain: 0.2,
                release: 1.0,
            },
        },
    ];

    // Musical sequences
    let sequences = [
        ("C Major Scale", vec![60, 62, 64, 65, 67, 69, 71, 72]),
        ("C Minor Scale", vec![60, 62, 63, 65, 67, 68, 70, 72]),
        ("C Major Arpeggio", vec![60, 64, 67, 72, 67, 64]),
        ("C Minor Arpeggio", vec![60, 63, 67, 72, 67, 63]),
        ("Pentatonic Riff", vec![60, 62, 65, 67, 69, 67, 65, 62]),
    ];

    let chords = [
        ("C Major", vec![60, 64, 67]),
        ("F Major", vec![65, 69, 72]),
        ("G Major", vec![67, 71, 74]),
        ("Am", vec![57, 60, 64]),
        ("C7", vec![60, 64, 67, 70]),
        ("Dm7", vec![62, 65, 69, 72]),
    ];

    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║          Polyphonic MIDI Synthesizer                         ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();
    println!("Commands:");
    println!("  [note numbers]  Play notes (e.g., '60' or '60 64 67')");
    println!("  preset [1-5]    Switch preset (1=Pluck, 2=Pad, 3=Bass, 4=Lead, 5=Bell)");
    println!("  seq [1-5]       Play sequence (1-5)");
    println!("  chord [1-6]     Play chord (1-6)");
    println!("  progression     Play I-V-vi-IV progression");
    println!("  help            Show this help");
    println!("  quit            Exit");
    println!();

    let engine = TuttiEngine::builder().sample_rate(44100.0).build()?;
    let current_preset = Arc::new(AtomicUsize::new(3)); // Start with Lead

    // Create single synth with initial preset
    let synth_node = engine.graph(|net| {
        let midi_registry = net.midi_registry().clone();
        let preset = presets[3]; // Start with Lead

        let mut synth = PolySynth::midi(44100.0, 16, midi_registry);
        synth.set_waveform(preset.waveform);
        synth.set_envelope(preset.envelope);

        let node_id = net.add(Box::new(synth));
        net.pipe_output(node_id);
        node_id
    });

    engine.transport().play();

    println!("Current preset: {}", presets[3].name);
    println!();

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let trimmed = input.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();

        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "quit" | "exit" | "q" => break,

            "help" | "h" => {
                println!();
                println!("Available presets:");
                for (i, preset) in presets.iter().enumerate() {
                    println!(
                        "  {} - {} ({:?}, A:{:.3} D:{:.3} S:{:.1} R:{:.3})",
                        i + 1,
                        preset.name,
                        preset.waveform,
                        preset.envelope.attack,
                        preset.envelope.decay,
                        preset.envelope.sustain,
                        preset.envelope.release
                    );
                }
                println!();
                println!("Available sequences:");
                for (i, (name, _)) in sequences.iter().enumerate() {
                    println!("  {} - {}", i + 1, name);
                }
                println!();
                println!("Available chords:");
                for (i, (name, _)) in chords.iter().enumerate() {
                    println!("  {} - {}", i + 1, name);
                }
                println!();
            }

            "preset" | "p" => {
                if parts.len() < 2 {
                    println!("Usage: preset [1-5]");
                    continue;
                }

                if let Ok(preset_num) = parts[1].parse::<usize>() {
                    if preset_num >= 1 && preset_num <= presets.len() {
                        let preset = presets[preset_num - 1];
                        current_preset.store(preset_num - 1, Ordering::Relaxed);

                        // Update synth parameters using node_mut with downcasting
                        engine.graph(|net| {
                            let node = net.node_mut(synth_node);
                            let any_node = <dyn AudioUnit>::as_any_mut(node);
                            if let Some(synth) = any_node.downcast_mut::<PolySynth>() {
                                synth.set_waveform(preset.waveform);
                                synth.set_envelope(preset.envelope);
                            }
                        });

                        println!(
                            "✓ Switched to preset: {} ({:?})",
                            preset.name, preset.waveform
                        );
                    } else {
                        println!("Invalid preset number. Use 1-{}", presets.len());
                    }
                } else {
                    println!("Invalid preset number");
                }
            }

            "seq" | "sequence" | "s" => {
                if parts.len() < 2 {
                    println!("Usage: seq [1-5]");
                    continue;
                }

                if let Ok(seq_num) = parts[1].parse::<usize>() {
                    if seq_num >= 1 && seq_num <= sequences.len() {
                        let (name, notes) = &sequences[seq_num - 1];
                        println!("Playing sequence: {}", name);

                        for &note in notes {
                            let event = MidiEvent::note_on_builder(note, 100).build();
                            engine.queue_midi(synth_node, &[event]);
                            std::thread::sleep(std::time::Duration::from_millis(200));

                            let off_event = MidiEvent::note_off_builder(note).build();
                            engine.queue_midi(synth_node, &[off_event]);
                        }
                    } else {
                        println!("Invalid sequence number. Use 1-{}", sequences.len());
                    }
                } else {
                    println!("Invalid sequence number");
                }
            }

            "chord" | "c" => {
                if parts.len() < 2 {
                    println!("Usage: chord [1-6]");
                    continue;
                }

                if let Ok(chord_num) = parts[1].parse::<usize>() {
                    if chord_num >= 1 && chord_num <= chords.len() {
                        let (name, notes) = &chords[chord_num - 1];
                        println!("Playing chord: {}", name);

                        let events: Vec<MidiEvent> = notes
                            .iter()
                            .map(|&note| MidiEvent::note_on_builder(note, 100).build())
                            .collect();

                        engine.queue_midi(synth_node, &events);
                        std::thread::sleep(std::time::Duration::from_secs(2));

                        let off_events: Vec<MidiEvent> = notes
                            .iter()
                            .map(|&note| MidiEvent::note_off_builder(note).build())
                            .collect();

                        engine.queue_midi(synth_node, &off_events);
                    } else {
                        println!("Invalid chord number. Use 1-{}", chords.len());
                    }
                } else {
                    println!("Invalid chord number");
                }
            }

            "progression" | "prog" => {
                println!("Playing I-V-vi-IV progression (C-G-Am-F)");
                let progression = [
                    ("C", vec![60, 64, 67]),  // I
                    ("G", vec![67, 71, 74]),  // V
                    ("Am", vec![57, 60, 64]), // vi
                    ("F", vec![65, 69, 72]),  // IV
                ];

                for (name, notes) in &progression {
                    println!("  {}", name);

                    let events: Vec<MidiEvent> = notes
                        .iter()
                        .map(|&note| MidiEvent::note_on_builder(note, 100).build())
                        .collect();

                    engine.queue_midi(synth_node, &events);
                    std::thread::sleep(std::time::Duration::from_millis(1500));

                    let off_events: Vec<MidiEvent> = notes
                        .iter()
                        .map(|&note| MidiEvent::note_off_builder(note).build())
                        .collect();

                    engine.queue_midi(synth_node, &off_events);
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }

            _ => {
                // Try parsing as MIDI note numbers
                let notes: Vec<u8> = parts
                    .iter()
                    .filter_map(|s| s.parse::<u8>().ok())
                    .filter(|&n| n <= 127)
                    .collect();

                if notes.is_empty() {
                    println!("Invalid command. Type 'help' for available commands.");
                    continue;
                }

                let preset_idx = current_preset.load(Ordering::Relaxed);

                // Create MIDI note-on events
                let midi_events: Vec<MidiEvent> = notes
                    .iter()
                    .map(|&note| MidiEvent::note_on_builder(note, 100).build())
                    .collect();

                engine.queue_midi(synth_node, &midi_events);

                // Show what's playing
                let note_names = notes
                    .iter()
                    .map(|&n| {
                        let note_name = [
                            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
                        ];
                        let octave = (n / 12) as i32 - 1;
                        let name_idx = (n % 12) as usize;
                        format!("{}{}", note_name[name_idx], octave)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                println!("♪ Playing: {}", note_names);

                // Sustain based on preset
                let sustain_time = if presets[preset_idx].envelope.sustain > 0.5 {
                    2000 // Pad/sustained sounds
                } else {
                    800 // Pluck/percussive sounds
                };

                std::thread::sleep(std::time::Duration::from_millis(sustain_time));

                // Send note-off events
                let note_off_events: Vec<MidiEvent> = notes
                    .iter()
                    .map(|&note| MidiEvent::note_off_builder(note).build())
                    .collect();
                engine.queue_midi(synth_node, &note_off_events);
            }
        }
    }

    println!("Goodbye!");
    Ok(())
}
