//! Integration tests for tutti-midi-io.
//!
//! These tests exercise multi-component workflows without hardware MIDI devices.

use std::time::Instant;
use tutti_midi_io::{
    cc::{CCMappingManager, CCTarget},
    midi_output_channel, MidiEvent, MidiOutputAggregator, MidiSystem, ParsedMidiFile, PortType,
};

// ---------------------------------------------------------------------------
// 1. MidiSystem end-to-end: ports + CC mapping + output collector
// ---------------------------------------------------------------------------

/// Build a full system, push events through ports, verify they flow correctly.
#[test]
fn test_midi_system_port_flow() {
    let midi = MidiSystem::builder()
        .cc_mapping()
        .output_collector()
        .build()
        .unwrap();

    // Create ports
    let input_idx = midi.create_input_port("Keyboard");
    let output_idx = midi.create_output_port("Synth Out");

    // Get port manager and push events through input
    let pm = midi.port_manager();
    let producer = pm.get_input_producer_handle(input_idx).unwrap();

    // Simulate hardware callback: push Note On + CC
    let note_on = MidiEvent::note_on(0, 0, 60, 100);
    let cc_msg = MidiEvent::control_change(10, 0, 7, 127);
    assert!(producer.push(note_on, Instant::now()));
    assert!(producer.push(cc_msg, Instant::now()));

    // Simulate audio thread: read all inputs
    let events = pm.cycle_start_read_all_inputs(512, Instant::now(), 44100.0);
    assert_eq!(events.len(), 2);
    assert!(events[0].1.is_note_on());
    assert_eq!(events[0].1.note(), Some(60));
    assert_eq!(events[1].1.note(), None); // CC has no note

    // Write a response to output port
    let note_off = MidiEvent::note_off(256, 0, 60, 0);
    assert!(pm.write_output_event(output_idx, note_off));

    // Flush outputs
    let output = pm.cycle_end_flush_all_outputs();
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].0, output_idx);
    assert!(output[0].1.is_note_off());
}

/// Port deactivation: events from inactive ports are not read.
#[test]
fn test_inactive_port_skipped_in_cycle() {
    let midi = MidiSystem::builder().build().unwrap();

    let port_a = midi.create_input_port("Active");
    let port_b = midi.create_input_port("Inactive");

    let pm = midi.port_manager();
    let handle_a = pm.get_input_producer_handle(port_a).unwrap();
    let handle_b = pm.get_input_producer_handle(port_b).unwrap();

    handle_a.push(MidiEvent::note_on(0, 0, 60, 100), Instant::now());
    handle_b.push(MidiEvent::note_on(0, 0, 72, 100), Instant::now());

    // Deactivate port B
    pm.set_port_active(PortType::Input, port_b, false);

    let events = pm.cycle_start_read_all_inputs(512, Instant::now(), 44100.0);
    assert_eq!(events.len(), 1, "Only active port events should appear");
    assert_eq!(events[0].1.note(), Some(60));
}

// ---------------------------------------------------------------------------
// 2. CC mapping: add → process → learn → process
// ---------------------------------------------------------------------------

/// End-to-end CC mapping workflow: add mapping, process CC, verify targets.
#[test]
fn test_cc_mapping_process_flow() {
    let midi = MidiSystem::builder().cc_mapping().build().unwrap();
    let cc_mgr = midi.cc_manager().unwrap();

    // Add a mapping: CC7 on channel 0 → MasterVolume [0.0, 1.0]
    cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

    // Process CC7 = 127 on channel 0
    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(result.targets.len(), 1);
    assert_eq!(result.targets[0].0, CCTarget::MasterVolume);
    assert!((result.targets[0].1 - 1.0).abs() < 0.01);

    // Process CC7 = 0 on channel 0
    let result = cc_mgr.process_cc(0, 7, 0);
    assert_eq!(result.targets.len(), 1);
    assert!((result.targets[0].1 - 0.0).abs() < 0.01);

    // Wrong channel → no targets
    let result = cc_mgr.process_cc(1, 7, 64);
    assert!(result.targets.is_empty());

    // Wrong CC number → no targets
    let result = cc_mgr.process_cc(0, 11, 64);
    assert!(result.targets.is_empty());
}

/// MIDI learn workflow: start learn → send CC → verify mapping created → use mapping.
#[test]
fn test_cc_learn_workflow() {
    let cc_mgr = CCMappingManager::new();

    // Start learn for Tempo target
    cc_mgr.start_learn(CCTarget::Tempo, 60.0, 200.0, None);
    assert!(cc_mgr.is_learning());
    assert_eq!(cc_mgr.get_learn_target(), Some(CCTarget::Tempo));

    // Send any CC → should complete learn
    let result = cc_mgr.process_cc(2, 11, 64);
    assert!(result.learn_completed.is_some());
    assert!(!cc_mgr.is_learning());

    // Now the learned mapping should work: CC11 on channel 2 → Tempo
    let result = cc_mgr.process_cc(2, 11, 127);
    assert_eq!(result.targets.len(), 1);
    assert_eq!(result.targets[0].0, CCTarget::Tempo);
    assert!((result.targets[0].1 - 200.0).abs() < 0.1);

    let result = cc_mgr.process_cc(2, 11, 0);
    assert_eq!(result.targets.len(), 1);
    assert!((result.targets[0].1 - 60.0).abs() < 0.1);
}

/// Learn with channel filter: only completes when the correct channel is seen.
#[test]
fn test_cc_learn_channel_filter() {
    let cc_mgr = CCMappingManager::new();

    // Learn only on channel 5
    cc_mgr.start_learn(CCTarget::MasterVolume, 0.0, 1.0, Some(5));
    assert!(cc_mgr.is_learning());

    // Wrong channel → still learning
    let result = cc_mgr.process_cc(0, 74, 64);
    assert!(result.learn_completed.is_none());
    assert!(cc_mgr.is_learning());

    // Correct channel → learn completes
    let result = cc_mgr.process_cc(5, 74, 64);
    assert!(result.learn_completed.is_some());
    assert!(!cc_mgr.is_learning());
}

/// Multiple mappings on same CC: both fire.
#[test]
fn test_cc_multiple_mappings_same_cc() {
    let cc_mgr = CCMappingManager::new();

    cc_mgr.add_mapping(None, 7, CCTarget::MasterVolume, 0.0, 1.0);
    cc_mgr.add_mapping(None, 7, CCTarget::TrackVolume(0), 0.0, 0.8);

    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(result.targets.len(), 2, "Both mappings should fire");
}

// ---------------------------------------------------------------------------
// 3. Output collector: multi-producer aggregation
// ---------------------------------------------------------------------------

/// Multiple producers push events, aggregator drains all.
#[test]
fn test_output_collector_multi_producer() {
    let midi = MidiSystem::builder().output_collector().build().unwrap();
    let aggregator = midi.output_collector().unwrap();

    // Create two producer/consumer pairs
    let (mut prod1, cons1) = midi_output_channel();
    let (mut prod2, cons2) = midi_output_channel();

    aggregator.add_consumer(cons1);
    aggregator.add_consumer(cons2);

    // Push from different producers
    prod1.push(MidiEvent::note_on(0, 0, 60, 100));
    prod1.push(MidiEvent::note_on(10, 0, 64, 80));
    prod2.push(MidiEvent::note_on(0, 1, 72, 120));

    // Drain all
    let events = aggregator.drain_all();
    assert_eq!(events.len(), 3);

    // After drain, should be empty
    assert!(!aggregator.has_pending());
    let events = aggregator.drain_all();
    assert_eq!(events.len(), 0);
}

/// Standalone aggregator test with capacity limits.
#[test]
fn test_output_collector_overflow() {
    use tutti_midi_io::midi_output_channel_with_capacity;

    let aggregator = MidiOutputAggregator::new();
    let (mut prod, cons) = midi_output_channel_with_capacity(4);
    aggregator.add_consumer(cons);

    // Fill to capacity
    for i in 0..4 {
        assert!(prod.push(MidiEvent::note_on(0, 0, 60 + i as u8, 100)));
    }
    // Next push should fail
    assert!(!prod.push(MidiEvent::note_on(0, 0, 70, 100)));

    // Drain frees space
    let events = aggregator.drain_all();
    assert_eq!(events.len(), 4);

    // Can push again
    assert!(prod.push(MidiEvent::note_on(0, 0, 80, 100)));
    let events = aggregator.drain_all();
    assert_eq!(events.len(), 1);
}

// ---------------------------------------------------------------------------
// 4. MIDI file parsing
// ---------------------------------------------------------------------------

/// Construct a minimal valid MIDI file in memory and parse it.
fn make_midi_file_bytes(ticks_per_beat: u16, tempo_us: u32, track_events: &[u8]) -> Vec<u8> {
    let track_len = track_events.len() as u32 + 4; // +4 for end-of-track
    let mut data = Vec::new();

    // MThd header
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&6u32.to_be_bytes()); // header length
    data.extend_from_slice(&0u16.to_be_bytes()); // format 0
    data.extend_from_slice(&1u16.to_be_bytes()); // 1 track
    data.extend_from_slice(&ticks_per_beat.to_be_bytes());

    // MTrk
    data.extend_from_slice(b"MTrk");

    // Track length: tempo meta event + user events + end-of-track
    let tempo_event = [
        0x00,
        0xFF,
        0x51,
        0x03,
        (tempo_us >> 16) as u8,
        (tempo_us >> 8) as u8,
        tempo_us as u8,
    ];
    let total_track_len = tempo_event.len() as u32 + track_len;
    data.extend_from_slice(&total_track_len.to_be_bytes());

    // Tempo meta event
    data.extend_from_slice(&tempo_event);

    // User events
    data.extend_from_slice(track_events);

    // End of track
    data.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);

    data
}

/// Parse a MIDI file with a note on/off pair and verify events.
#[test]
fn test_midi_file_parse_note_pair() {
    // 480 ticks per beat, 120 BPM (500000 us/qn)
    let track_events = [
        // Delta=0, Note On ch0, note 60, vel 100
        0x00, 0x90, 60, 100, // Delta=480 (one beat), Note Off ch0, note 60, vel 0
        0x83, 0x60, // VLQ for 480
        0x80, 60, 0,
    ];

    let data = make_midi_file_bytes(480, 500_000, &track_events);
    let file = ParsedMidiFile::parse(&data).unwrap();

    assert_eq!(file.ticks_per_beat, 480);
    assert!((file.tempo_bpm - 120.0).abs() < 0.1);
    assert_eq!(file.events.len(), 2);

    // First event: note on at beat 0
    assert!((file.events[0].time_beats - 0.0).abs() < 0.001);
    assert_eq!(file.events[0].channel, 0);
    match file.events[0].event {
        tutti_midi_io::MidiEventType::NoteOn { note, velocity } => {
            assert_eq!(note, 60);
            assert_eq!(velocity, 100);
        }
        _ => panic!("Expected NoteOn, got {:?}", file.events[0].event),
    }

    // Second event: note off at beat 1
    assert!((file.events[1].time_beats - 1.0).abs() < 0.001);
    match file.events[1].event {
        tutti_midi_io::MidiEventType::NoteOff { note, .. } => {
            assert_eq!(note, 60);
        }
        _ => panic!("Expected NoteOff, got {:?}", file.events[1].event),
    }

    // Duration
    assert!((file.duration_beats - 1.0).abs() < 0.001);
}

/// NoteOn with velocity 0 should be treated as NoteOff.
#[test]
fn test_midi_file_velocity_zero_is_note_off() {
    let track_events = [
        0x00, 0x90, 60, 100, // Note On
        0x83, 0x60, // delta 480
        0x90, 60, 0, // Note On vel=0 → should become NoteOff
    ];

    let data = make_midi_file_bytes(480, 500_000, &track_events);
    let file = ParsedMidiFile::parse(&data).unwrap();

    assert_eq!(file.events.len(), 2);
    match file.events[1].event {
        tutti_midi_io::MidiEventType::NoteOff { note, velocity } => {
            assert_eq!(note, 60);
            assert_eq!(velocity, 0);
        }
        _ => panic!(
            "Expected NoteOff from vel=0, got {:?}",
            file.events[1].event
        ),
    }
}

/// Parse MIDI file with control change and pitch bend events.
#[test]
fn test_midi_file_cc_and_pitch_bend() {
    let track_events = [
        // CC: delta=0, ch0, CC7 = 100
        0x00, 0xB0, 7, 100, // Pitch Bend: delta=0, ch0, LSB=0, MSB=64 (center = 8192)
        0x00, 0xE0, 0x00, 0x40,
    ];

    let data = make_midi_file_bytes(480, 500_000, &track_events);
    let file = ParsedMidiFile::parse(&data).unwrap();

    assert_eq!(file.events.len(), 2);

    match file.events[0].event {
        tutti_midi_io::MidiEventType::ControlChange { controller, value } => {
            assert_eq!(controller, 7);
            assert_eq!(value, 100);
        }
        _ => panic!("Expected ControlChange"),
    }

    match file.events[1].event {
        tutti_midi_io::MidiEventType::PitchBend { value } => {
            assert_eq!(value, 0, "Center pitch bend should be 0");
        }
        _ => panic!("Expected PitchBend"),
    }
}

/// get_events_in_range returns correct slice.
#[test]
fn test_midi_file_events_in_range() {
    // Create 4 notes at beats 0, 1, 2, 3
    let track_events = [
        0x00, 0x90, 60, 100, // beat 0
        0x83, 0x60, 0x90, 64, 100, // beat 1
        0x83, 0x60, 0x90, 67, 100, // beat 2
        0x83, 0x60, 0x90, 72, 100, // beat 3
    ];

    let data = make_midi_file_bytes(480, 500_000, &track_events);
    let file = ParsedMidiFile::parse(&data).unwrap();

    assert_eq!(file.events.len(), 4);

    // Range [0.5, 2.5) should include beats 1 and 2
    let slice = file.get_events_in_range(0.5, 2.5);
    assert_eq!(slice.len(), 2);
    match slice[0].event {
        tutti_midi_io::MidiEventType::NoteOn { note, .. } => assert_eq!(note, 64),
        _ => panic!("Expected NoteOn"),
    }
    match slice[1].event {
        tutti_midi_io::MidiEventType::NoteOn { note, .. } => assert_eq!(note, 67),
        _ => panic!("Expected NoteOn"),
    }

    // Range [0, 0.5) should include only beat 0
    let slice = file.get_events_in_range(0.0, 0.5);
    assert_eq!(slice.len(), 1);

    // Range [4.0, 5.0) should be empty
    let slice = file.get_events_in_range(4.0, 5.0);
    assert_eq!(slice.len(), 0);
}

// ---------------------------------------------------------------------------
// 5. MidiSystem event creation + port injection
// ---------------------------------------------------------------------------

/// Create events via MidiSystem helpers, inject into ports, read back.
#[test]
fn test_event_creation_and_port_injection() {
    let midi = MidiSystem::builder().build().unwrap();
    let port = midi.create_input_port("Test");
    let pm = midi.port_manager();
    let handle = pm.get_input_producer_handle(port).unwrap();

    // Create events via fluent API
    let events = [
        midi.note_on(0, 60, 100),
        midi.cc(0, 74, 64),
        midi.pitch_bend(0, 8192),
        midi.note_off(0, 60, 0),
    ];

    for e in &events {
        assert!(handle.push(*e, Instant::now()));
    }

    // Read back
    let read = pm.cycle_start_read_all_inputs(512, Instant::now(), 44100.0);
    assert_eq!(read.len(), 4);
    assert!(read[0].1.is_note_on());
    assert!(read[3].1.is_note_off());
}

/// Events with frame offsets for sample-accurate timing.
#[test]
fn test_frame_offset_events() {
    let midi = MidiSystem::builder().build().unwrap();

    let e1 = midi.note_on_at(0, 0, 60, 100);
    let e2 = midi.note_on_at(128, 0, 64, 80);
    let e3 = midi.note_off_at(256, 0, 60, 0);

    assert_eq!(e1.frame_offset, 0);
    assert_eq!(e2.frame_offset, 128);
    assert_eq!(e3.frame_offset, 256);

    // Verify note content
    assert_eq!(e1.note(), Some(60));
    assert_eq!(e2.note(), Some(64));
    assert_eq!(e3.note(), Some(60));
}

// ---------------------------------------------------------------------------
// 6. Clone semantics: cloned system shares state
// ---------------------------------------------------------------------------

#[test]
fn test_midi_system_clone_shares_state() {
    let midi1 = MidiSystem::builder().cc_mapping().build().unwrap();
    let midi2 = midi1.clone();

    // Create port via midi1
    midi1.create_input_port("Shared Port");

    // Should be visible from midi2
    let ports = midi2.list_input_ports();
    assert_eq!(ports.len(), 1);
    assert_eq!(ports[0].name, "Shared Port");

    // CC mapping added via midi1 should be visible from midi2
    let cc1 = midi1.cc_manager().unwrap();
    cc1.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

    let cc2 = midi2.cc_manager().unwrap();
    assert_eq!(cc2.get_all_mappings().len(), 1);
}

// ---------------------------------------------------------------------------
// 7. MIDI file → port pipeline
// ---------------------------------------------------------------------------

/// Parse a MIDI file and inject its events into ports.
#[test]
fn test_midi_file_to_port_pipeline() {
    let track_events = [
        0x00, 0x90, 60, 100, // beat 0: Note On
        0x83, 0x60, 0x80, 60, 0, // beat 1: Note Off
        0x00, 0x90, 64, 80, // beat 1: Note On
        0x83, 0x60, 0x80, 64, 0, // beat 2: Note Off
    ];

    let data = make_midi_file_bytes(480, 500_000, &track_events);
    let file = ParsedMidiFile::parse(&data).unwrap();

    let midi = MidiSystem::builder().build().unwrap();
    let port = midi.create_input_port("MIDI File");
    let pm = midi.port_manager();
    let handle = pm.get_input_producer_handle(port).unwrap();

    // Inject all events from the file
    for timed_event in &file.events {
        let event = match timed_event.event {
            tutti_midi_io::MidiEventType::NoteOn { note, velocity } => {
                MidiEvent::note_on(0, timed_event.channel, note, velocity)
            }
            tutti_midi_io::MidiEventType::NoteOff { note, velocity } => {
                MidiEvent::note_off(0, timed_event.channel, note, velocity)
            }
            tutti_midi_io::MidiEventType::ControlChange {
                controller, value, ..
            } => MidiEvent::control_change(0, timed_event.channel, controller, value),
            tutti_midi_io::MidiEventType::PitchBend { value } => {
                let unsigned = (value as i32 + 8192) as u16;
                MidiEvent::pitch_bend(0, timed_event.channel, unsigned)
            }
            tutti_midi_io::MidiEventType::ProgramChange { .. } => continue,
        };
        assert!(handle.push(event, Instant::now()));
    }

    // Read all
    let events = pm.cycle_start_read_all_inputs(512, Instant::now(), 44100.0);
    assert_eq!(events.len(), 4);

    // Verify ordering: on, off, on, off
    assert!(events[0].1.is_note_on());
    assert!(events[1].1.is_note_off());
    assert!(events[2].1.is_note_on());
    assert!(events[3].1.is_note_off());
}

// ---------------------------------------------------------------------------
// 8. CC mapping enable/disable/remove
// ---------------------------------------------------------------------------

#[test]
fn test_cc_mapping_enable_disable_remove() {
    let cc_mgr = CCMappingManager::new();
    let id = cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

    // Initially enabled
    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(result.targets.len(), 1);

    // Disable
    assert!(cc_mgr.set_mapping_enabled(id, false));
    let result = cc_mgr.process_cc(0, 7, 127);
    assert!(
        result.targets.is_empty(),
        "Disabled mapping should not fire"
    );

    // Re-enable
    assert!(cc_mgr.set_mapping_enabled(id, true));
    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(result.targets.len(), 1);

    // Remove
    assert!(cc_mgr.remove_mapping(id));
    let result = cc_mgr.process_cc(0, 7, 127);
    assert!(result.targets.is_empty(), "Removed mapping should not fire");

    // Remove again → false
    assert!(!cc_mgr.remove_mapping(id));
}

// ---------------------------------------------------------------------------
// 9. CC mapping: find_mappings, clear_all, get_mapping, cancel_learn
// ---------------------------------------------------------------------------

/// find_mappings returns only mappings matching channel+CC, not others.
#[test]
fn test_cc_find_mappings_filters_correctly() {
    let cc_mgr = CCMappingManager::new();

    // Add 3 mappings: two on CC7 (different channels), one on CC11
    cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);
    cc_mgr.add_mapping(Some(1), 7, CCTarget::TrackVolume(0), 0.0, 1.0);
    cc_mgr.add_mapping(Some(0), 11, CCTarget::Tempo, 60.0, 200.0);

    // find_mappings(ch0, cc7) should return only the first
    let found = cc_mgr.find_mappings(0, 7);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.target, CCTarget::MasterVolume);

    // find_mappings(ch1, cc7) should return only the second
    let found = cc_mgr.find_mappings(1, 7);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.target, CCTarget::TrackVolume(0));

    // find_mappings(ch0, cc11) should return only the third
    let found = cc_mgr.find_mappings(0, 11);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.target, CCTarget::Tempo);

    // find_mappings(ch5, cc99) should return nothing
    let found = cc_mgr.find_mappings(5, 99);
    assert!(found.is_empty());
}

/// find_mappings includes "any channel" mappings for every channel query.
#[test]
fn test_cc_find_mappings_any_channel() {
    let cc_mgr = CCMappingManager::new();

    // Any-channel mapping on CC7
    cc_mgr.add_mapping(None, 7, CCTarget::MasterVolume, 0.0, 1.0);
    // Channel-specific mapping on CC7
    cc_mgr.add_mapping(Some(3), 7, CCTarget::TrackVolume(1), 0.0, 1.0);

    // Querying channel 3 should find both
    let found = cc_mgr.find_mappings(3, 7);
    assert_eq!(found.len(), 2);

    // Querying channel 0 should find only the any-channel one
    let found = cc_mgr.find_mappings(0, 7);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.target, CCTarget::MasterVolume);
}

/// Disabled mappings are excluded from find_mappings.
#[test]
fn test_cc_find_mappings_excludes_disabled() {
    let cc_mgr = CCMappingManager::new();

    let id = cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);
    assert_eq!(cc_mgr.find_mappings(0, 7).len(), 1);

    cc_mgr.set_mapping_enabled(id, false);
    assert!(
        cc_mgr.find_mappings(0, 7).is_empty(),
        "Disabled mapping should be excluded"
    );
}

/// clear_all removes everything; subsequent process_cc produces no targets.
#[test]
fn test_cc_clear_all_removes_everything() {
    let cc_mgr = CCMappingManager::new();

    cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);
    cc_mgr.add_mapping(Some(1), 11, CCTarget::Tempo, 60.0, 200.0);
    assert_eq!(cc_mgr.get_all_mappings().len(), 2);

    cc_mgr.clear_all();
    assert_eq!(cc_mgr.get_all_mappings().len(), 0);

    // process_cc should produce nothing
    let result = cc_mgr.process_cc(0, 7, 127);
    assert!(result.targets.is_empty());
}

/// get_mapping returns correct mapping, or None for invalid ID.
#[test]
fn test_cc_get_mapping_by_id() {
    let cc_mgr = CCMappingManager::new();

    let id = cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

    let mapping = cc_mgr.get_mapping(id);
    assert!(mapping.is_some());
    let m = mapping.unwrap();
    assert_eq!(m.cc_number, 7);
    assert_eq!(m.target, CCTarget::MasterVolume);
    assert_eq!(m.channel, Some(0));

    // Non-existent ID
    assert!(cc_mgr.get_mapping(999).is_none());
}

/// cancel_learn exits learn mode without creating a mapping.
#[test]
fn test_cc_cancel_learn() {
    let cc_mgr = CCMappingManager::new();

    cc_mgr.start_learn(CCTarget::Tempo, 60.0, 200.0, None);
    assert!(cc_mgr.is_learning());
    assert_eq!(cc_mgr.get_learn_target(), Some(CCTarget::Tempo));

    cc_mgr.cancel_learn();
    assert!(!cc_mgr.is_learning());
    assert!(cc_mgr.get_learn_target().is_none());

    // No mapping should have been created
    assert!(cc_mgr.get_all_mappings().is_empty());

    // process_cc should not complete any learn
    let result = cc_mgr.process_cc(0, 7, 64);
    assert!(result.learn_completed.is_none());
    assert!(result.targets.is_empty());
}

/// set_mapping_enabled returns false for non-existent ID.
#[test]
fn test_cc_set_enabled_nonexistent() {
    let cc_mgr = CCMappingManager::new();
    assert!(!cc_mgr.set_mapping_enabled(999, false));
}

/// During learn mode, normal mappings don't fire — learn intercepts the CC.
#[test]
fn test_cc_learn_blocks_normal_processing() {
    let cc_mgr = CCMappingManager::new();

    // Add a real mapping first
    cc_mgr.add_mapping(Some(0), 7, CCTarget::MasterVolume, 0.0, 1.0);

    // Start learn
    cc_mgr.start_learn(CCTarget::Tempo, 60.0, 200.0, None);

    // Process CC7 ch0 — this should complete learn, NOT fire the volume mapping
    let result = cc_mgr.process_cc(0, 7, 127);
    assert!(result.learn_completed.is_some(), "Learn should complete");
    assert!(
        result.targets.is_empty(),
        "Normal mappings should not fire during learn"
    );

    // Now the learn is done — process again, the original + learned mapping should fire
    let result = cc_mgr.process_cc(0, 7, 127);
    assert_eq!(
        result.targets.len(),
        2,
        "Both original and learned mapping should fire"
    );
}

// ---------------------------------------------------------------------------
// 10. MidiSystem: channel_pressure, poly_pressure, channel/pitch_bend clamping
// ---------------------------------------------------------------------------

/// MidiSystem::channel_pressure creates a valid ChannelPressure event.
#[test]
fn test_midi_system_channel_pressure() {
    let midi = MidiSystem::builder().build().unwrap();

    let event = midi.channel_pressure(3, 100);
    assert_eq!(event.channel_num(), 3);
    match event.msg {
        tutti_midi_io::ChannelVoiceMsg::ChannelPressure { pressure } => {
            assert_eq!(pressure, 100);
        }
        _ => panic!("Expected ChannelPressure, got {:?}", event.msg),
    }
}

/// MidiSystem::poly_pressure creates a valid PolyPressure event.
#[test]
fn test_midi_system_poly_pressure() {
    let midi = MidiSystem::builder().build().unwrap();

    let event = midi.poly_pressure(5, 72, 80);
    assert_eq!(event.channel_num(), 5);
    match event.msg {
        tutti_midi_io::ChannelVoiceMsg::PolyPressure { note, pressure } => {
            assert_eq!(note, 72);
            assert_eq!(pressure, 80);
        }
        _ => panic!("Expected PolyPressure, got {:?}", event.msg),
    }
}

/// MidiSystem channel clamping: channel > 15 should clamp to 15.
#[test]
fn test_midi_system_channel_clamping() {
    let midi = MidiSystem::builder().build().unwrap();

    // note_on with channel 200
    let event = midi.note_on(200, 60, 100);
    assert_eq!(event.channel_num(), 15, "Channel should clamp to 15");

    // note_off with overflow channel
    let event = midi.note_off(255, 60, 0);
    assert_eq!(event.channel_num(), 15);

    // cc with overflow channel
    let event = midi.cc(128, 7, 127);
    assert_eq!(event.channel_num(), 15);

    // pitch_bend with overflow channel
    let event = midi.pitch_bend(16, 8192);
    assert_eq!(event.channel_num(), 15);

    // channel_pressure with overflow channel
    let event = midi.channel_pressure(99, 100);
    assert_eq!(event.channel_num(), 15);

    // poly_pressure with overflow channel
    let event = midi.poly_pressure(200, 60, 80);
    assert_eq!(event.channel_num(), 15);
}

/// MidiSystem pitch_bend clamping: value > 16383 should clamp to 16383.
#[test]
fn test_midi_system_pitch_bend_value_clamping() {
    let midi = MidiSystem::builder().build().unwrap();

    let event = midi.pitch_bend(0, 20000);
    match event.msg {
        tutti_midi_io::ChannelVoiceMsg::PitchBend { bend } => {
            assert_eq!(bend, 16383, "Pitch bend should clamp to 16383");
        }
        _ => panic!("Expected PitchBend"),
    }

    // Normal value should pass through
    let event = midi.pitch_bend(0, 8192);
    match event.msg {
        tutti_midi_io::ChannelVoiceMsg::PitchBend { bend } => {
            assert_eq!(bend, 8192);
        }
        _ => panic!("Expected PitchBend"),
    }
}

// ---------------------------------------------------------------------------
// 11. Port manager: port_info, is_port_active, write_event_to_port inactive
// ---------------------------------------------------------------------------

/// port_info returns correct info or None for invalid index.
#[test]
fn test_port_info_by_index() {
    let midi = MidiSystem::builder().build().unwrap();

    let idx = midi.create_input_port("MyInput");
    let info = midi.port_info(PortType::Input, idx);
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.name, "MyInput");

    // Also accessible via MidiSystem
    let idx2 = midi.create_output_port("MyOutput");
    let out_info = midi.port_info(PortType::Output, idx2);
    assert!(out_info.is_some());
    assert_eq!(out_info.unwrap().name, "MyOutput");

    // Invalid index should return None (index 999 doesn't exist)
    assert!(midi.port_info(PortType::Input, 999).is_none());
}

/// is_port_active reflects activation state correctly.
#[test]
fn test_port_active_toggle() {
    let midi = MidiSystem::builder().build().unwrap();
    let pm = midi.port_manager();

    let port = midi.create_input_port("Toggle");
    assert!(pm.is_port_active(PortType::Input, port));

    pm.set_port_active(PortType::Input, port, false);
    assert!(!pm.is_port_active(PortType::Input, port));

    pm.set_port_active(PortType::Input, port, true);
    assert!(pm.is_port_active(PortType::Input, port));

    // Non-existent port returns false
    assert!(!pm.is_port_active(PortType::Input, 999));
}

/// write_event_to_port returns false for inactive output port.
#[test]
fn test_write_event_to_inactive_output_port() {
    let midi = MidiSystem::builder().build().unwrap();
    let pm = midi.port_manager();

    let out_idx = midi.create_output_port("Inactive Out");
    let event = MidiEvent::note_on(0, 0, 60, 100);

    // Active → should succeed
    assert!(pm.write_event_to_port(out_idx, event));

    // Deactivate the output port
    pm.set_port_active(PortType::Output, out_idx, false);

    // Inactive → should return false
    assert!(!pm.write_event_to_port(out_idx, event));
}

/// write_event_to_port returns false for non-existent port.
#[test]
fn test_write_event_to_nonexistent_port() {
    let midi = MidiSystem::builder().build().unwrap();
    let pm = midi.port_manager();

    let event = MidiEvent::note_on(0, 0, 60, 100);
    assert!(!pm.write_event_to_port(999, event));
}

/// output_port_count returns correct count.
#[test]
fn test_output_port_count() {
    let midi = MidiSystem::builder().build().unwrap();
    let pm = midi.port_manager();

    assert_eq!(pm.output_port_count(), 0);

    midi.create_output_port("Out1");
    assert_eq!(pm.output_port_count(), 1);

    midi.create_output_port("Out2");
    assert_eq!(pm.output_port_count(), 2);
}

/// list_output_ports returns output ports only.
#[test]
fn test_list_output_ports() {
    let midi = MidiSystem::builder().build().unwrap();

    midi.create_input_port("Input1");
    midi.create_output_port("Output1");
    midi.create_output_port("Output2");

    let outputs = midi.list_output_ports();
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().all(|p| p.name.starts_with("Output")));

    let inputs = midi.list_input_ports();
    assert_eq!(inputs.len(), 1);
}

/// list_ports returns both input and output ports.
#[test]
fn test_list_all_ports() {
    let midi = MidiSystem::builder().build().unwrap();

    midi.create_input_port("In1");
    midi.create_input_port("In2");
    midi.create_output_port("Out1");

    let all = midi.list_ports();
    assert_eq!(all.len(), 3);
}

// ---------------------------------------------------------------------------
// 12. MPE handle: reset, expression
// ---------------------------------------------------------------------------

#[cfg(feature = "mpe")]
mod mpe_integration {
    use tutti_midi_io::{MidiSystem, MpeMode, MpeZoneConfig};

    /// MpeHandle::reset clears channel allocations and expression state.
    #[test]
    fn test_mpe_handle_reset() {
        let midi = MidiSystem::builder()
            .mpe(MpeMode::LowerZone(MpeZoneConfig::lower(5)))
            .build()
            .unwrap();

        let mpe = midi.mpe();

        // Allocate a channel
        let ch = mpe.allocate_channel(60);
        assert!(ch.is_some());
        assert!(mpe.get_channel(60).is_some());

        // Reset should clear everything
        mpe.reset();
        assert!(
            mpe.get_channel(60).is_none(),
            "Channel allocation should be cleared"
        );
    }

    /// MpeHandle::expression returns shared expression state.
    #[test]
    fn test_mpe_handle_expression_returns_arc() {
        let midi = MidiSystem::builder()
            .mpe(MpeMode::LowerZone(MpeZoneConfig::lower(5)))
            .build()
            .unwrap();

        let mpe = midi.mpe();
        let expr = mpe.expression();
        assert!(expr.is_some(), "Should return Some when MPE is enabled");

        // Also accessible via MidiSystem::expression()
        let expr2 = midi.expression();
        assert!(expr2.is_some());
    }

    /// Disabled MPE handle returns None for expression.
    #[test]
    fn test_mpe_disabled_expression_is_none() {
        let midi = MidiSystem::builder().build().unwrap();
        assert!(midi.expression().is_none());
    }
}

// ---------------------------------------------------------------------------
// 13. Unified MIDI 2.0 event flow through ports
// ---------------------------------------------------------------------------

#[cfg(feature = "midi2")]
mod midi2_integration {
    use tutti_midi_io::{Midi2Event, MidiEvent, MidiSystem, UnifiedMidiEvent};

    /// Push a MIDI 2.0 event into a port and read it back.
    #[test]
    fn test_push_midi2_event_to_port() {
        let midi = MidiSystem::builder().build().unwrap();
        let port = midi.create_input_port("MIDI2 Input");

        let event = Midi2Event::note_on(
            0,
            midi2::prelude::u4::new(0),
            midi2::prelude::u4::new(3),
            midi2::prelude::u7::new(60),
            32768,
        );

        assert!(midi.push_midi2_event(port, event));

        // Read back via unified API
        let pm = midi.port_manager();
        let events = pm.cycle_start_read_all_unified_inputs(512);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, port);
        assert!(events[0].1.is_note_on());
        assert_eq!(events[0].1.channel(), 3);
        assert_eq!(events[0].1.note(), Some(60));
    }

    /// Push a unified V1 event and read it back.
    #[test]
    fn test_push_unified_v1_event() {
        let midi = MidiSystem::builder().build().unwrap();
        let port = midi.create_input_port("Unified Input");

        let v1 = MidiEvent::note_on(0, 5, 72, 100);
        let unified = UnifiedMidiEvent::V1(v1);

        assert!(midi.push_unified_event(port, unified));

        let pm = midi.port_manager();
        let events = pm.cycle_start_read_all_unified_inputs(512);
        assert_eq!(events.len(), 1);
        assert!(events[0].1.is_v1());
        assert_eq!(events[0].1.note(), Some(72));
        assert_eq!(events[0].1.velocity(), Some(100));
    }

    /// Push to non-existent port returns false.
    #[test]
    fn test_push_midi2_to_nonexistent_port() {
        let midi = MidiSystem::builder().build().unwrap();

        let event = Midi2Event::note_on(
            0,
            midi2::prelude::u4::new(0),
            midi2::prelude::u4::new(0),
            midi2::prelude::u7::new(60),
            32768,
        );

        assert!(!midi.push_midi2_event(999, event));
    }

    /// Mixed V1 and V2 events coexist in unified stream.
    #[test]
    fn test_mixed_v1_v2_unified_stream() {
        let midi = MidiSystem::builder().build().unwrap();
        let port = midi.create_input_port("Mixed");

        // Push V1
        let v1 = UnifiedMidiEvent::V1(MidiEvent::note_on(0, 0, 60, 100));
        assert!(midi.push_unified_event(port, v1));

        // Push V2
        let v2 = UnifiedMidiEvent::V2(Midi2Event::note_on(
            10,
            midi2::prelude::u4::new(0),
            midi2::prelude::u4::new(0),
            midi2::prelude::u7::new(72),
            65535,
        ));
        assert!(midi.push_unified_event(port, v2));

        let pm = midi.port_manager();
        let events = pm.cycle_start_read_all_unified_inputs(512);
        assert_eq!(events.len(), 2);
        assert!(events[0].1.is_v1());
        assert!(events[1].1.is_v2());
        assert_eq!(events[0].1.note(), Some(60));
        assert_eq!(events[1].1.note(), Some(72));
    }

    /// Unified event to_midi1/to_midi2 conversions work end-to-end.
    #[test]
    fn test_unified_event_conversions() {
        // V1 → to_midi2
        let v1 = MidiEvent::note_on(0, 3, 64, 80);
        let unified = UnifiedMidiEvent::V1(v1);
        let as_midi2 = unified.to_midi2().unwrap();
        assert_eq!(as_midi2.channel(), 3);
        assert_eq!(as_midi2.note(), Some(64));

        // V2 → to_midi1: use a known velocity that round-trips cleanly
        // MIDI 2.0 velocity 0x8000 maps to approximately 64 in MIDI 1.0
        let v2 = Midi2Event::note_on(
            0,
            midi2::prelude::u4::new(0),
            midi2::prelude::u4::new(5),
            midi2::prelude::u7::new(60),
            0xFFFF, // Max velocity → should map to 127
        );
        let unified = UnifiedMidiEvent::V2(v2);
        let as_midi1 = unified.to_midi1().unwrap();
        assert_eq!(as_midi1.channel_num(), 5);
        assert_eq!(as_midi1.note(), Some(60));
        assert_eq!(as_midi1.velocity(), Some(127));
    }
}

// ---------------------------------------------------------------------------
// 14. Regression: interleaved port creation targets correct port
// ---------------------------------------------------------------------------

/// When input and output ports are interleaved, set_port_active must affect
/// the correct port (not a wrong one due to index space confusion).
#[test]
fn test_interleaved_port_active_targets_correct_port() {
    let midi = MidiSystem::builder().build().unwrap();
    let pm = midi.port_manager();

    let input_0 = midi.create_input_port("Input");
    let output_0 = midi.create_output_port("Output");

    // Deactivate the output port (index 0 in output space)
    pm.set_port_active(PortType::Output, output_0, false);

    // Input port should still be active
    assert!(
        pm.is_port_active(PortType::Input, input_0),
        "Deactivating output should not affect input"
    );
    // Output port should be inactive
    assert!(
        !pm.is_port_active(PortType::Output, output_0),
        "Output port should be inactive"
    );

    // Reactivate output, deactivate input
    pm.set_port_active(PortType::Output, output_0, true);
    pm.set_port_active(PortType::Input, input_0, false);

    assert!(
        !pm.is_port_active(PortType::Input, input_0),
        "Input port should be inactive"
    );
    assert!(
        pm.is_port_active(PortType::Output, output_0),
        "Output port should still be active"
    );
}

/// get_port_info with PortType correctly distinguishes input vs output at same index.
#[test]
fn test_port_info_distinguishes_type() {
    let midi = MidiSystem::builder().build().unwrap();

    let input_0 = midi.create_input_port("Input Zero");
    let output_0 = midi.create_output_port("Output Zero");

    // Both have index 0, but different types
    assert_eq!(input_0, 0);
    assert_eq!(output_0, 0);

    let input_info = midi.port_info(PortType::Input, 0).unwrap();
    assert_eq!(input_info.name, "Input Zero");

    let output_info = midi.port_info(PortType::Output, 0).unwrap();
    assert_eq!(output_info.name, "Output Zero");
}
