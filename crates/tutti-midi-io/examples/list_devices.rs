use tutti_midi_io::MidiSystem;

fn main() {
    let midi = MidiSystem::builder().io().build().unwrap();

    println!("=== MIDI Input Devices ===");
    let devices = midi.list_devices();
    if devices.is_empty() {
        println!("  (none found)");
    }
    for dev in &devices {
        println!("  [{}] {}", dev.index, dev.name);
    }

    println!("\n=== MIDI Output Devices ===");
    let midi_out = midir::MidiOutput::new("list-devices").unwrap();
    let ports = midi_out.ports();
    if ports.is_empty() {
        println!("  (none found)");
    }
    for (i, port) in ports.iter().enumerate() {
        let name = midi_out.port_name(port).unwrap_or_else(|_| "?".into());
        println!("  [{}] {}", i, name);
    }
}
