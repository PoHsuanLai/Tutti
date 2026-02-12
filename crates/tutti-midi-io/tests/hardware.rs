//! Hardware integration tests using macOS IAC Driver loopback.
//!
//! Requires IAC Driver to be enabled in Audio MIDI Setup.
//! All tests are `#[ignore]` so CI doesn't fail without hardware.
//!
//! Run with:
//!   cargo test -p tutti-midi-io --test hardware -- --ignored --test-threads=1
//!   cargo test -p tutti-midi-io --test hardware --features mpe -- --ignored --test-threads=1
//!   cargo test -p tutti-midi-io --test hardware --features "midi2,mpe" -- --ignored --test-threads=1

#![cfg(feature = "midi-io")]

use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tutti_midi_io::{
    ChannelVoiceMsg, ControlChange, MidiEvent, MidiHandle, MidiOutputMessage, MidiSystem,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const IAC_SETTLE: Duration = Duration::from_millis(200);
const SEND_READ_DELAY: Duration = Duration::from_millis(100);
const BURST_DELAY: Duration = Duration::from_millis(300);

/// Build a MidiSystem with I/O, connect both input and output to the IAC Driver.
fn setup_iac() -> MidiSystem {
    setup_iac_inner(false, false)
}

fn setup_iac_with_cc() -> MidiSystem {
    setup_iac_inner(true, false)
}

#[cfg(feature = "mpe")]
fn setup_iac_with_mpe() -> MidiSystem {
    let midi = MidiSystem::builder()
        .io()
        .mpe(tutti_midi_io::MpeMode::LowerZone(
            tutti_midi_io::MpeZoneConfig::lower(15),
        ))
        .build()
        .expect("Failed to build MidiSystem with MPE");
    connect_iac(&midi);
    midi
}

fn setup_iac_inner(cc: bool, _mpe: bool) -> MidiSystem {
    let mut builder = MidiSystem::builder().io();
    if cc {
        builder = builder.cc_mapping();
    }
    let midi = builder.build().expect("Failed to build MidiSystem");
    connect_iac(&midi);
    midi
}

fn connect_iac(midi: &MidiSystem) {
    // Connect input
    midi.connect_device_by_name("IAC")
        .expect("IAC Driver not found. Enable it in Audio MIDI Setup → Window → Show MIDI Studio → IAC Driver → Device is online");

    // Connect output
    let output_mgr = midi
        .output_manager()
        .expect("Output manager not enabled");
    output_mgr
        .connect_by_name("IAC")
        .expect("IAC Driver output not found");

    // Wait for async connection threads
    thread::sleep(IAC_SETTLE);
    assert!(
        midi.is_device_connected(),
        "Should be connected to IAC input"
    );

    // Drain stale events (loop until empty to handle OS-level buffering)
    let pm = midi.port_manager();
    for _ in 0..5 {
        let stale = pm.cycle_start_read_all_inputs(512);
        if stale.is_empty() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Send events via closure, wait, then read all input events.
fn send_and_read(
    midi: &MidiSystem,
    send_fn: impl FnOnce(&MidiSystem),
) -> Vec<MidiEvent> {
    let pm = midi.port_manager();
    // Drain stale events twice with a gap to catch any in-flight messages
    let _ = pm.cycle_start_read_all_inputs(512);
    thread::sleep(Duration::from_millis(20));
    let _ = pm.cycle_start_read_all_inputs(512);

    send_fn(midi);
    thread::sleep(SEND_READ_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    events.iter().map(|(_, e)| *e).collect()
}

/// Send raw bytes via output manager, wait, read.
fn send_raw_and_read(midi: &MidiSystem, msg: MidiOutputMessage) -> Vec<MidiEvent> {
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    let output_mgr = midi.output_manager().unwrap();
    output_mgr.send_message(msg);
    thread::sleep(SEND_READ_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    events.iter().map(|(_, e)| *e).collect()
}

// ===========================================================================
// Group 1: Basic Message Types
// ===========================================================================

#[test]
#[ignore]
fn test_note_on_loopback() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 60, 100).unwrap();
    });
    assert_eq!(events.len(), 1, "Expected 1 event, got {}", events.len());
    assert!(events[0].is_note_on());
    assert_eq!(events[0].note(), Some(60));
    assert_eq!(events[0].velocity(), Some(100));
}

#[test]
#[ignore]
fn test_note_off_loopback() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_note_off(0, 60, 64).unwrap();
    });
    assert_eq!(events.len(), 1);
    assert!(events[0].is_note_off());
    assert_eq!(events[0].note(), Some(60));
}

#[test]
#[ignore]
fn test_cc_loopback() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 74, 127).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::ControlChange { control } => match control {
            ControlChange::CC { control: cc, value } => {
                assert_eq!(cc, 74);
                assert_eq!(value, 127);
            }
            _ => panic!("Expected CC, got {:?}", control),
        },
        _ => panic!("Expected ControlChange, got {:?}", events[0].msg),
    }
}

#[test]
#[ignore]
fn test_pitch_bend_center() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_pitch_bend(0, 0).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::PitchBend { bend } => {
            assert_eq!(bend, 8192, "Center pitch bend should be 8192");
        }
        _ => panic!("Expected PitchBend"),
    }
}

#[test]
#[ignore]
fn test_pitch_bend_max_up() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_pitch_bend(0, 8191).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::PitchBend { bend } => {
            assert_eq!(bend, 16383, "Max up pitch bend should be 16383");
        }
        _ => panic!("Expected PitchBend"),
    }
}

#[test]
#[ignore]
fn test_pitch_bend_max_down() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_pitch_bend(0, -8192).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::PitchBend { bend } => {
            assert_eq!(bend, 0, "Max down pitch bend should be 0");
        }
        _ => panic!("Expected PitchBend"),
    }
}

#[test]
#[ignore]
fn test_program_change_loopback() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send_program_change(0, 42).unwrap();
    });
    // macOS Core MIDI may prepend Bank Select CC messages (CC 0, CC 32) before
    // a Program Change. Find the actual ProgramChange event.
    assert!(!events.is_empty(), "Expected at least 1 event");
    let pc = events
        .iter()
        .find(|e| matches!(e.msg, ChannelVoiceMsg::ProgramChange { .. }))
        .expect("Expected a ProgramChange event in the received events");
    match pc.msg {
        ChannelVoiceMsg::ProgramChange { program } => {
            assert_eq!(program, 42);
        }
        _ => unreachable!(),
    }
}

#[test]
#[ignore]
fn test_send_event_loopback() {
    let midi = setup_iac();
    let event = MidiEvent::note_on(0, 0, 72, 80);
    let events = send_and_read(&midi, |m| {
        m.send_event(&event).unwrap();
    });
    assert_eq!(events.len(), 1);
    assert!(events[0].is_note_on());
    assert_eq!(events[0].note(), Some(72));
    assert_eq!(events[0].velocity(), Some(80));
}

// ===========================================================================
// Group 2: Fluent Builder API
// ===========================================================================

#[test]
#[ignore]
fn test_fluent_single() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send().note_on(0, 60, 100);
    });
    assert_eq!(events.len(), 1);
    assert!(events[0].is_note_on());
    assert_eq!(events[0].note(), Some(60));
}

#[test]
#[ignore]
fn test_fluent_chain_two() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send().note_on(0, 60, 100).cc(0, 74, 64);
    });
    assert_eq!(events.len(), 2, "Expected 2 events, got {}", events.len());
    assert!(events[0].is_note_on());
    match events[1].msg {
        ChannelVoiceMsg::ControlChange { .. } => {}
        _ => panic!("Expected CC as second event"),
    }
}

#[test]
#[ignore]
fn test_fluent_chain_three() {
    let midi = setup_iac();
    let events = send_and_read(&midi, |m| {
        m.send()
            .note_on(0, 60, 100)
            .pitch_bend(0, 0)
            .note_off(0, 60, 0);
    });
    assert_eq!(events.len(), 3, "Expected 3 events, got {}", events.len());
    assert!(events[0].is_note_on());
    assert!(matches!(events[1].msg, ChannelVoiceMsg::PitchBend { .. }));
    assert!(events[2].is_note_off());
}

// ===========================================================================
// Group 3: MidiHandle Wrapper
// ===========================================================================

#[test]
#[ignore]
fn test_handle_send() {
    let midi = setup_iac();
    let handle = MidiHandle::new(Some(Arc::new(midi)));
    assert!(handle.is_enabled());
    assert!(handle.inner().is_some());

    // Send via handle's fluent API, read via the inner system's port manager
    let inner = handle.inner().unwrap();
    let pm = inner.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);
    thread::sleep(Duration::from_millis(20));
    let _ = pm.cycle_start_read_all_inputs(512);

    handle.send().note_on(0, 60, 100);
    thread::sleep(SEND_READ_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    assert_eq!(events.len(), 1, "Expected 1 event via handle send");
    assert!(events[0].1.is_note_on());
    assert_eq!(events[0].1.note(), Some(60));
}

#[test]
#[ignore]
fn test_handle_connect_disconnect() {
    let midi = MidiSystem::builder().io().build().unwrap();
    let handle = MidiHandle::new(Some(Arc::new(midi)));

    // Connect via handle
    handle.connect_device_by_name("IAC").unwrap();
    thread::sleep(IAC_SETTLE);

    // Verify via inner
    let inner = handle.inner().unwrap();
    assert!(inner.is_device_connected());
    assert!(inner.connected_device_name().unwrap().contains("IAC"));

    // Disconnect
    handle.disconnect_device();
    thread::sleep(IAC_SETTLE);
    assert!(!inner.is_device_connected());
}

#[test]
#[ignore]
fn test_handle_list_devices() {
    let midi = MidiSystem::builder().io().build().unwrap();
    let handle = MidiHandle::new(Some(Arc::new(midi)));

    let devices = handle.list_devices();
    assert!(!devices.is_empty(), "Should find at least IAC Driver");
    assert!(
        devices.iter().any(|d| d.name.contains("IAC")),
        "IAC Driver should be in the list"
    );
}

// ===========================================================================
// Group 4: Connection Lifecycle
// ===========================================================================

#[test]
#[ignore]
fn test_connect_state_queries() {
    let midi = MidiSystem::builder().io().build().unwrap();

    // Before connect
    assert!(!midi.is_device_connected());
    assert!(midi.connected_device_name().is_none());
    assert!(midi.connected_port_index().is_none());

    // Connect
    midi.connect_device_by_name("IAC").unwrap();
    thread::sleep(IAC_SETTLE);

    assert!(midi.is_device_connected());
    let name = midi.connected_device_name().unwrap();
    assert!(name.contains("IAC"), "Device name should contain IAC, got: {name}");
    assert!(midi.connected_port_index().is_some());

    // Disconnect
    midi.disconnect_device();
    thread::sleep(IAC_SETTLE);

    assert!(!midi.is_device_connected());
    assert!(midi.connected_device_name().is_none());
    assert!(midi.connected_port_index().is_none());
}

#[test]
#[ignore]
fn test_reconnect_works() {
    let midi = setup_iac();

    // Disconnect input
    midi.disconnect_device();
    thread::sleep(IAC_SETTLE);
    assert!(!midi.is_device_connected());

    // Reconnect input
    midi.connect_device_by_name("IAC").unwrap();
    thread::sleep(IAC_SETTLE);
    assert!(midi.is_device_connected());

    // Drain stale events
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    // Send and verify loopback still works
    midi.send_note_on(0, 60, 100).unwrap();
    thread::sleep(SEND_READ_DELAY);
    let events = pm.cycle_start_read_all_inputs(512);
    assert!(!events.is_empty(), "Should receive events after reconnect");
    assert!(events[0].1.is_note_on());
}

#[test]
#[ignore]
fn test_send_after_output_disconnect() {
    let midi = setup_iac();
    let output_mgr = midi.output_manager().unwrap();

    // Disconnect output
    output_mgr.disconnect();
    thread::sleep(IAC_SETTLE);

    // send_note_on should not panic (message silently dropped)
    let result = midi.send_note_on(0, 60, 100);
    assert!(result.is_ok(), "send_note_on should not error even with disconnected output");

    // No events should arrive
    thread::sleep(SEND_READ_DELAY);
    let pm = midi.port_manager();
    let events = pm.cycle_start_read_all_inputs(512);
    assert_eq!(events.len(), 0, "No events should arrive after output disconnect");
}

#[test]
#[ignore]
fn test_no_events_after_input_disconnect() {
    let midi = setup_iac();

    // Verify baseline works
    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 60, 100).unwrap();
    });
    assert_eq!(events.len(), 1);

    // Disconnect input
    midi.disconnect_device();
    thread::sleep(IAC_SETTLE);

    // Send another message — output still connected but input disconnected
    midi.send_note_on(0, 64, 100).unwrap();
    thread::sleep(SEND_READ_DELAY);

    // The original port still exists but no new events should be pushed
    // (midir callback no longer running)
    let pm = midi.port_manager();
    let events = pm.cycle_start_read_all_inputs(512);
    assert_eq!(events.len(), 0, "No events after input disconnect");
}

// ===========================================================================
// Group 5: Edge Cases
// ===========================================================================

#[test]
#[ignore]
fn test_channel_0_and_15() {
    let midi = setup_iac();

    // Channel 0
    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 60, 100).unwrap();
    });
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].channel_num(), 0);

    // Channel 15
    let events = send_and_read(&midi, |m| {
        m.send_note_on(15, 60, 100).unwrap();
    });
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].channel_num(), 15);
}

#[test]
#[ignore]
fn test_note_0_and_127() {
    let midi = setup_iac();

    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 0, 100).unwrap();
    });
    assert_eq!(events[0].note(), Some(0));

    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 127, 100).unwrap();
    });
    assert_eq!(events[0].note(), Some(127));
}

#[test]
#[ignore]
fn test_velocity_1_and_127() {
    let midi = setup_iac();

    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 60, 1).unwrap();
    });
    assert!(events[0].is_note_on());
    assert_eq!(events[0].velocity(), Some(1));

    let events = send_and_read(&midi, |m| {
        m.send_note_on(0, 60, 127).unwrap();
    });
    assert_eq!(events[0].velocity(), Some(127));
}

#[test]
#[ignore]
fn test_velocity_0_is_note_off() {
    let midi = setup_iac();
    let events = send_raw_and_read(&midi, MidiOutputMessage::note_on(0, 60, 0));
    assert_eq!(events.len(), 1);
    assert!(
        events[0].is_note_off(),
        "Note on with velocity 0 should be treated as note off"
    );
}

#[test]
#[ignore]
fn test_cc_number_boundaries() {
    let midi = setup_iac();

    // CC 0 (Bank Select MSB)
    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 0, 64).unwrap();
    });
    assert_eq!(events.len(), 1);
    // CC 0 might be parsed as BankSelect by midi-msg, check it round-trips
    match events[0].msg {
        ChannelVoiceMsg::ControlChange { .. } => {} // Any CC variant is fine
        _ => panic!("Expected ControlChange"),
    }

    // CC 119 (highest regular CC; 120-127 are Channel Mode messages, not CCs)
    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 119, 64).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::ControlChange { .. } => {}
        _ => panic!("Expected ControlChange"),
    }
}

#[test]
#[ignore]
fn test_cc_value_boundaries() {
    let midi = setup_iac();

    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 74, 0).unwrap();
    });
    assert_eq!(events.len(), 1);
    match events[0].msg {
        ChannelVoiceMsg::ControlChange { control } => {
            if let ControlChange::CC { value, .. } = control {
                assert_eq!(value, 0);
            }
        }
        _ => panic!("Expected ControlChange"),
    }

    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 74, 127).unwrap();
    });
    match events[0].msg {
        ChannelVoiceMsg::ControlChange { control } => {
            if let ControlChange::CC { value, .. } = control {
                assert_eq!(value, 127);
            }
        }
        _ => panic!("Expected ControlChange"),
    }
}

#[test]
#[ignore]
fn test_pitch_bend_sweep() {
    let midi = setup_iac();

    // (signed_input, expected_14bit_unsigned)
    let test_cases: [(i16, u16); 5] = [
        (-8192, 0),
        (-4096, 4096),
        (0, 8192),
        (4096, 12288),
        (8191, 16383),
    ];

    for (signed, expected) in &test_cases {
        let events = send_and_read(&midi, |m| {
            m.send_pitch_bend(0, *signed).unwrap();
        });
        assert_eq!(events.len(), 1, "Missing event for bend={signed}");
        match events[0].msg {
            ChannelVoiceMsg::PitchBend { bend } => {
                assert_eq!(
                    bend, *expected,
                    "Pitch bend {signed} should map to {expected}, got {bend}"
                );
            }
            _ => panic!("Expected PitchBend for input {signed}"),
        }
    }
}

#[test]
#[ignore]
fn test_all_16_channels() {
    let midi = setup_iac();

    // Drain
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    // Send note on each channel
    for ch in 0..16u8 {
        midi.send_note_on(ch, 60, 100).unwrap();
    }
    thread::sleep(BURST_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    assert_eq!(events.len(), 16, "Expected 16 events, got {}", events.len());

    let mut channels: Vec<u8> = events.iter().map(|(_, e)| e.channel_num()).collect();
    channels.sort();
    let expected: Vec<u8> = (0..16).collect();
    assert_eq!(channels, expected, "All 16 channels should be present");
}

// ===========================================================================
// Group 6: Stress
// ===========================================================================

#[test]
#[ignore]
fn test_100_event_burst() {
    let midi = setup_iac();
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    let output_mgr = midi.output_manager().unwrap();
    for note in 0..100u8 {
        output_mgr.send_message(MidiOutputMessage::note_on(0, note.min(127), 80));
    }
    thread::sleep(BURST_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    assert!(
        events.len() >= 95,
        "Expected >=95 events, got {} (some OS jitter allowed)",
        events.len()
    );
    assert!(
        events.iter().all(|(_, e)| e.is_note_on()),
        "All events should be note on"
    );
}

#[test]
#[ignore]
fn test_interleaved_types() {
    let midi = setup_iac();
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    let output_mgr = midi.output_manager().unwrap();
    for _ in 0..20 {
        output_mgr.send_message(MidiOutputMessage::note_on(0, 60, 100));
        output_mgr.send_message(MidiOutputMessage::control_change(0, 74, 64));
        output_mgr.send_message(MidiOutputMessage::pitch_bend(0, 0));
        output_mgr.send_message(MidiOutputMessage::note_off(0, 60, 0));
    }
    thread::sleep(BURST_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    assert!(
        events.len() >= 76,
        "Expected >=76 of 80 events, got {}",
        events.len()
    );
}

#[test]
#[ignore]
fn test_sustained_multi_channel() {
    let midi = setup_iac();
    let pm = midi.port_manager();
    let _ = pm.cycle_start_read_all_inputs(512);

    let output_mgr = midi.output_manager().unwrap();
    for ch in 0..16u8 {
        for note in [60u8, 64, 67, 72] {
            output_mgr.send_message(MidiOutputMessage::note_on(ch, note, 100));
        }
    }
    thread::sleep(BURST_DELAY);

    let events = pm.cycle_start_read_all_inputs(512);
    assert_eq!(
        events.len(),
        64,
        "Expected 64 events (16 ch * 4 notes), got {}",
        events.len()
    );
}

// ===========================================================================
// Group 7: CC Mapping via IAC
// ===========================================================================

#[test]
#[ignore]
fn test_cc_mapping_from_iac() {
    let midi = setup_iac_with_cc();
    let cc_mgr = midi.cc_manager().unwrap();

    // Add mapping: CC7 ch0 → MasterVolume [0.0, 1.0]
    cc_mgr.add_mapping(
        Some(0),
        7,
        tutti_midi_io::CCTarget::MasterVolume,
        0.0,
        1.0,
    );

    // Send CC7=127 on ch0 via IAC
    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 7, 127).unwrap();
    });
    assert_eq!(events.len(), 1);

    // Process the received CC through the mapping manager
    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(result.targets.len(), 1);
    assert_eq!(result.targets[0].0, tutti_midi_io::CCTarget::MasterVolume);
    assert!((result.targets[0].1 - 1.0).abs() < 0.01);
}

#[test]
#[ignore]
fn test_midi_learn_from_iac() {
    let midi = setup_iac_with_cc();
    let cc_mgr = midi.cc_manager().unwrap();

    // Start learning for Tempo [60, 200]
    cc_mgr.start_learn(tutti_midi_io::CCTarget::Tempo, 60.0, 200.0, None);
    assert!(cc_mgr.is_learning());

    // Send CC11=64 on ch2 via IAC
    let events = send_and_read(&midi, |m| {
        m.send_cc(2, 11, 64).unwrap();
    });
    assert_eq!(events.len(), 1);

    // Process — should complete learn
    let result = cc_mgr.process_cc(2, 11, 64);
    assert!(result.learn_completed.is_some());
    assert!(!cc_mgr.is_learning());

    // Now send CC11=127 on ch2 and process
    let events = send_and_read(&midi, |m| {
        m.send_cc(2, 11, 127).unwrap();
    });
    assert_eq!(events.len(), 1);
    let result = cc_mgr.process_cc(2, 11, 127);
    assert_eq!(result.targets.len(), 1);
    assert_eq!(result.targets[0].0, tutti_midi_io::CCTarget::Tempo);
    assert!((result.targets[0].1 - 200.0).abs() < 0.5);
}

#[test]
#[ignore]
fn test_cc_mapping_channel_filter() {
    let midi = setup_iac_with_cc();
    let cc_mgr = midi.cc_manager().unwrap();

    // Mapping only on ch0
    cc_mgr.add_mapping(
        Some(0),
        7,
        tutti_midi_io::CCTarget::MasterVolume,
        0.0,
        1.0,
    );

    // Send on ch0 → should match
    let events = send_and_read(&midi, |m| {
        m.send_cc(0, 7, 100).unwrap();
    });
    assert_eq!(events.len(), 1);
    let result = cc_mgr.process_cc(0, 7, 100);
    assert_eq!(result.targets.len(), 1);

    // Send on ch1 → should not match
    let events = send_and_read(&midi, |m| {
        m.send_cc(1, 7, 100).unwrap();
    });
    assert_eq!(events.len(), 1);
    let result = cc_mgr.process_cc(1, 7, 100);
    assert!(result.targets.is_empty());
}

// ===========================================================================
// Group 8: MPE via IAC
// ===========================================================================

#[cfg(feature = "mpe")]
mod mpe_hw {
    use super::*;

    #[test]
    #[ignore]
    fn test_mpe_note_on_activates() {
        let midi = setup_iac_with_mpe();

        // Send note_on on ch1 (member channel in lower zone)
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));

        assert!(
            midi.mpe().is_note_active(60),
            "Note 60 should be active after note_on on member channel"
        );
    }

    #[test]
    #[ignore]
    fn test_mpe_pitch_bend_per_note() {
        let midi = setup_iac_with_mpe();

        // Note on ch1 note 60
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));

        // Pitch bend max on ch1
        midi.send_pitch_bend(1, 8191).unwrap();
        thread::sleep(Duration::from_millis(150));

        let bend = midi.mpe().pitch_bend(60);
        assert!(
            (bend - 1.0).abs() < 0.05,
            "Per-note pitch bend should be ~1.0, got {bend}"
        );
    }

    #[test]
    #[ignore]
    fn test_mpe_pressure() {
        let midi = setup_iac_with_mpe();

        // Note on ch1 note 60
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));

        // Channel pressure on ch1 = 127 (raw bytes: 0xD1, 127)
        let output_mgr = midi.output_manager().unwrap();
        output_mgr.send_message(MidiOutputMessage {
            bytes: vec![0xD1, 127],
        });
        thread::sleep(Duration::from_millis(150));

        let pressure = midi.mpe().pressure(60);
        assert!(
            (pressure - 1.0).abs() < 0.05,
            "Pressure should be ~1.0, got {pressure}"
        );
    }

    #[test]
    #[ignore]
    fn test_mpe_slide_cc74() {
        let midi = setup_iac_with_mpe();

        // Note on ch1 note 60
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));

        // CC74 = 127 on ch1
        midi.send_cc(1, 74, 127).unwrap();
        thread::sleep(Duration::from_millis(150));

        let slide = midi.mpe().slide(60);
        assert!(
            (slide - 1.0).abs() < 0.05,
            "Slide should be ~1.0, got {slide}"
        );
    }

    #[test]
    #[ignore]
    fn test_mpe_note_off_deactivates() {
        let midi = setup_iac_with_mpe();

        // Note on
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(midi.mpe().is_note_active(60));

        // Note off
        midi.send_note_off(1, 60, 0).unwrap();
        thread::sleep(Duration::from_millis(150));
        assert!(
            !midi.mpe().is_note_active(60),
            "Note 60 should be inactive after note_off"
        );
    }

    #[test]
    #[ignore]
    fn test_mpe_global_pitch_bend() {
        let midi = setup_iac_with_mpe();

        // Note on ch1
        midi.send_note_on(1, 60, 100).unwrap();
        thread::sleep(Duration::from_millis(150));

        // Global pitch bend on ch0 (master)
        midi.send_pitch_bend(0, 8191).unwrap();
        thread::sleep(Duration::from_millis(150));

        let global = midi.mpe().pitch_bend_global();
        assert!(
            (global - 1.0).abs() < 0.05,
            "Global pitch bend should be ~1.0, got {global}"
        );
    }
}

// ===========================================================================
// Group 9: MIDI 2.0 Conversion
// ===========================================================================

#[cfg(all(feature = "midi2", feature = "mpe"))]
mod midi2_hw {
    use super::*;

    #[test]
    #[ignore]
    fn test_midi1_to_midi2_note() {
        let midi = setup_iac();

        let events = send_and_read(&midi, |m| {
            m.send_note_on(0, 60, 100).unwrap();
        });
        assert_eq!(events.len(), 1);

        let midi2_event = midi.midi2().convert_to_midi2(&events[0]);
        assert!(midi2_event.is_some(), "Should convert to MIDI 2.0");
        let m2 = midi2_event.unwrap();
        assert!(m2.is_note_on());
        assert_eq!(m2.note(), Some(60));
        assert_eq!(m2.channel(), 0);

        // Velocity should be upsampled from 7-bit to 16-bit
        let vel16 = m2.velocity_16bit().unwrap();
        assert!(vel16 > 0, "Upsampled velocity should be > 0");
    }

    #[test]
    #[ignore]
    fn test_midi1_to_midi2_cc_roundtrip() {
        let midi = setup_iac();

        let events = send_and_read(&midi, |m| {
            m.send_cc(0, 7, 64).unwrap();
        });
        assert_eq!(events.len(), 1);

        let m2 = midi.midi2().convert_to_midi2(&events[0]);
        assert!(m2.is_some());

        // Convert back to MIDI 1.0
        let back = midi.midi2().convert_to_midi1(&m2.unwrap());
        assert!(back.is_some());
    }

    #[test]
    #[ignore]
    fn test_midi1_to_midi2_pitch_bend_roundtrip() {
        let midi = setup_iac();

        let events = send_and_read(&midi, |m| {
            m.send_pitch_bend(0, 0).unwrap(); // center
        });
        assert_eq!(events.len(), 1);

        let m2 = midi.midi2().convert_to_midi2(&events[0]);
        assert!(m2.is_some());

        let back = midi.midi2().convert_to_midi1(&m2.unwrap());
        assert!(back.is_some());
        let back = back.unwrap();
        match back.msg {
            ChannelVoiceMsg::PitchBend { bend } => {
                assert_eq!(bend, 8192, "Round-trip pitch bend center should be 8192");
            }
            _ => panic!("Expected PitchBend after round-trip"),
        }
    }

    #[test]
    #[ignore]
    fn test_unified_event_wrapping() {
        let midi = setup_iac();

        let events = send_and_read(&midi, |m| {
            m.send_note_on(5, 64, 80).unwrap();
        });
        assert_eq!(events.len(), 1);

        let unified = tutti_midi_io::UnifiedMidiEvent::V1(events[0]);
        assert!(unified.is_v1());
        assert!(!unified.is_v2());
        assert_eq!(unified.channel(), 5);
        assert_eq!(unified.note(), Some(64));
        assert!(unified.is_note_on());

        let norm = unified.velocity_normalized().unwrap();
        assert!((norm - 80.0 / 127.0).abs() < 0.02);
    }
}
