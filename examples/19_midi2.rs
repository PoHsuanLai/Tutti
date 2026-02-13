//! # 19 - MIDI 2.0
//!
//! Create high-resolution MIDI 2.0 events and convert between MIDI 1.0 and 2.0.
//!
//! **Concepts:** Midi2Event, 16-bit velocity, 32-bit controllers, per-note pitch bend, conversion
//!
//! ```bash
//! cargo run --example 19_midi2 --features midi,midi2
//! ```

use tutti::prelude::*;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().midi().build()?;
    let midi2 = engine.midi().midi2();

    // --- Note On with 16-bit velocity ---
    let note_on = midi2.note_on(60, 0.75, 0); // C4, 75% velocity, channel 0
    println!("MIDI 2.0 Note On:");
    println!("  Note:              {}", note_on.note().unwrap());
    println!("  Velocity (16-bit): {}", note_on.velocity_16bit().unwrap());
    println!(
        "  Velocity (norm):   {:.4}",
        note_on.velocity_normalized().unwrap()
    );
    println!("  Velocity (7-bit):  {}", note_on.velocity().unwrap());

    // --- Per-note pitch bend (MIDI 2.0 only) ---
    let bend = midi2.per_note_pitch_bend(60, 0.5, 0); // Half-step up on note 60
    println!("\nPer-note pitch bend:");
    match bend.message_type() {
        tutti::midi::Midi2MessageType::PerNotePitchBend { note, bend } => {
            println!("  Note: {note}, Bend (raw 32-bit): {bend:#010X}");
        }
        _ => unreachable!(),
    }

    // --- High-resolution CC ---
    let cc = midi2.control_change(74, 0.6, 0); // CC74 at 60%
    match cc.message_type() {
        tutti::midi::Midi2MessageType::ControlChange { controller, value } => {
            println!("\nControl Change:");
            println!("  Controller: {controller} (CC74 = Brightness/Slide)");
            println!("  Value (32-bit): {value:#010X}");
            println!("  Value (7-bit):  {}", midi2.cc_to_7bit(value));
        }
        _ => unreachable!(),
    }

    // --- MIDI 2.0 → MIDI 1.0 conversion (lossy) ---
    println!("\n--- Conversion: MIDI 2.0 → MIDI 1.0 ---");
    if let Some(midi1) = midi2.convert_to_midi1(&note_on) {
        println!(
            "  Note On → channel={}, note={}, velocity={}",
            midi1.channel_num(),
            midi1.note().unwrap(),
            midi1.velocity().unwrap()
        );
    }

    // Per-note pitch bend has no MIDI 1.0 equivalent
    let converted = midi2.convert_to_midi1(&bend);
    println!(
        "  Per-note pitch bend → {:?}",
        converted
            .map(|_| "converted")
            .unwrap_or("None (MIDI 2.0 only)")
    );

    // --- MIDI 1.0 → MIDI 2.0 conversion (upsampled) ---
    println!("\n--- Conversion: MIDI 1.0 → MIDI 2.0 ---");
    let midi1_event = MidiEvent::note_on(0, 0, 60, 100);
    if let Some(midi2_event) = midi2.convert_to_midi2(&midi1_event) {
        println!(
            "  Note On: velocity 100 (7-bit) → {} (16-bit)",
            midi2_event.velocity_16bit().unwrap()
        );
    }

    // --- Velocity resolution helpers ---
    println!("\n--- Velocity resolution ---");
    for v7 in [0u8, 1, 64, 100, 127] {
        let v16 = midi2.velocity_to_16bit(v7);
        let back = midi2.velocity_to_7bit(v16);
        println!("  7-bit {v7:>3} → 16-bit {v16:>5} → 7-bit {back:>3}");
    }

    Ok(())
}
