//! # 17 - MIDI Hardware
//!
//! Enumerate MIDI input devices and connect to hardware for receiving events.
//!
//! **Concepts:** Device enumeration, hardware connection, `midi-hardware` feature
//!
//! ```bash
//! cargo run --example 17_midi_hardware --features midi,midi-hardware
//! ```

use std::time::Duration;
use tutti::prelude::*;
use tutti::TuttiNet;

fn main() -> tutti::Result<()> {
    let engine = TuttiEngine::builder().midi().build()?;
    let midi = engine.midi();

    // List available MIDI input devices
    let devices = midi.list_devices();
    println!("MIDI input devices:");
    if devices.is_empty() {
        println!("  (none found — connect a MIDI controller and try again)");
        return Ok(());
    }
    for dev in &devices {
        println!("  [{}] {}", dev.index, dev.name);
    }

    // Connect to the first available device
    let device_name = &devices[0].name;
    println!("\nConnecting to: {device_name}");
    midi.connect_device_by_name(device_name)?;

    // Create a simple sine synth to hear incoming notes
    engine.graph_mut(|net: &mut TuttiNet| {
        let osc = sine_hz::<f32>(440.0) * 0.3;
        net.add(osc).master();
    });

    engine.transport().play();
    println!("Listening for MIDI input for 10 seconds...");
    println!("(Play notes on your MIDI controller)");
    std::thread::sleep(Duration::from_secs(10));

    // Disconnect
    midi.disconnect_device();
    println!("Disconnected.");

    Ok(())
}
