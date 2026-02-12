//! Hardware loopback test using macOS IAC Driver.
//!
//! Requires IAC Driver to be enabled in Audio MIDI Setup.
//! Sends MIDI messages out through IAC, receives them back, and verifies correctness.

use std::thread;
use std::time::Duration;
use tutti_midi_io::MidiSystem;

fn main() {
    // Build system with I/O enabled
    let midi = MidiSystem::builder().io().build().unwrap();

    // Check IAC Driver is available
    let inputs = midi.list_devices();
    let iac_input = inputs.iter().find(|d| d.name.contains("IAC"));
    if iac_input.is_none() {
        eprintln!("ERROR: IAC Driver not found. Enable it in Audio MIDI Setup.");
        std::process::exit(1);
    }
    let iac_input = iac_input.unwrap();
    println!("Found IAC input: [{}] {}", iac_input.index, iac_input.name);

    // Connect input to IAC Driver
    midi.connect_device(iac_input.index).unwrap();

    // Wait for connection to establish (async thread)
    thread::sleep(Duration::from_millis(200));
    assert!(midi.is_device_connected(), "Should be connected to IAC input");
    println!(
        "Connected input: {:?}",
        midi.connected_device_name().unwrap()
    );

    // Connect output to IAC Driver using midir directly
    // (MidiOutputManager connection is internal, so we use it through the manager)
    let output_mgr = midi.output_manager().unwrap();
    // Find IAC output
    let midi_out_temp = midir::MidiOutput::new("iac-test-find").unwrap();
    let ports = midi_out_temp.ports();
    let iac_out_idx = ports
        .iter()
        .enumerate()
        .find(|(_, p)| {
            midi_out_temp
                .port_name(p)
                .unwrap_or_default()
                .contains("IAC")
        })
        .map(|(i, _)| i);
    drop(midi_out_temp);

    if let Some(idx) = iac_out_idx {
        println!("Connecting output to IAC device index {}", idx);
        output_mgr.connect(idx).unwrap();
        thread::sleep(Duration::from_millis(200));
    } else {
        eprintln!("ERROR: IAC Driver output not found.");
        std::process::exit(1);
    }

    // Get the port manager to read events from the input port
    let pm = midi.port_manager();
    let input_port_idx = midi.connected_port_index().unwrap();
    println!("Input port index: {}", input_port_idx);

    // Drain any stale events
    let _ = pm.cycle_start_read_all_inputs(512);

    println!("\n=== Test 1: Note On ===");
    output_mgr.send_message(tutti_midi_io::MidiOutputMessage::note_on(0, 60, 100));
    thread::sleep(Duration::from_millis(100));
    let events = pm.cycle_start_read_all_inputs(512);
    if events.is_empty() {
        println!("  FAIL: No events received");
    } else {
        let e = &events[0].1;
        let ok = e.is_note_on() && e.note() == Some(60) && e.velocity() == Some(100);
        println!(
            "  {}: got {} event(s), first: note_on={}, note={:?}, vel={:?}",
            if ok { "PASS" } else { "FAIL" },
            events.len(),
            e.is_note_on(),
            e.note(),
            e.velocity()
        );
    }

    println!("\n=== Test 2: Note Off ===");
    output_mgr.send_message(tutti_midi_io::MidiOutputMessage::note_off(0, 60, 64));
    thread::sleep(Duration::from_millis(100));
    let events = pm.cycle_start_read_all_inputs(512);
    if events.is_empty() {
        println!("  FAIL: No events received");
    } else {
        let e = &events[0].1;
        let ok = e.is_note_off() && e.note() == Some(60);
        println!(
            "  {}: note_off={}, note={:?}",
            if ok { "PASS" } else { "FAIL" },
            e.is_note_off(),
            e.note()
        );
    }

    println!("\n=== Test 3: Control Change ===");
    output_mgr.send_message(tutti_midi_io::MidiOutputMessage::control_change(0, 74, 127));
    thread::sleep(Duration::from_millis(100));
    let events = pm.cycle_start_read_all_inputs(512);
    if events.is_empty() {
        println!("  FAIL: No events received");
    } else {
        let e = &events[0].1;
        println!("  Received: {:?}", e.msg);
        println!("  PASS (CC event received)");
    }

    println!("\n=== Test 4: Pitch Bend (center) ===");
    output_mgr.send_message(tutti_midi_io::MidiOutputMessage::pitch_bend(0, 0));
    thread::sleep(Duration::from_millis(100));
    let events = pm.cycle_start_read_all_inputs(512);
    if events.is_empty() {
        println!("  FAIL: No events received");
    } else {
        let e = &events[0].1;
        println!("  Received: {:?}", e.msg);
        println!("  PASS (Pitch bend event received)");
    }

    println!("\n=== Test 5: Rapid burst (10 notes) ===");
    for note in 60..70u8 {
        output_mgr.send_message(tutti_midi_io::MidiOutputMessage::note_on(0, note, 80));
    }
    thread::sleep(Duration::from_millis(200));
    let events = pm.cycle_start_read_all_inputs(512);
    let count = events.len();
    let all_note_on = events.iter().all(|(_, e)| e.is_note_on());
    println!(
        "  {}: received {}/10 events, all note_on={}",
        if count == 10 && all_note_on {
            "PASS"
        } else {
            "FAIL"
        },
        count,
        all_note_on
    );

    // Disconnect
    midi.disconnect_device();
    output_mgr.disconnect();

    println!("\nAll tests complete!");
}
