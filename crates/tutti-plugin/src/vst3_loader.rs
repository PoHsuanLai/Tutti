//! VST3 plugin loader
//!
//! This module handles loading and interfacing with VST3 plugins in the bridge server.

use crate::error::{BridgeError, LoadStage, Result};
use crate::protocol::{AudioBuffer, MidiEvent, PluginMetadata};
use libloading::Library;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// Import MIDI types for event conversion
use tutti_midi_io::{Channel, ChannelVoiceMsg, ControlChange};

/// Result codes from VST3
const K_RESULT_OK: i32 = 0;
const K_RESULT_TRUE: i32 = 0;

/// VST3 module entry point function signature
type GetPluginFactoryFn = unsafe extern "system" fn() -> *mut c_void;

/// Sample sizes
const K_SAMPLE_32: i32 = 0;
#[allow(dead_code)]
const K_SAMPLE_64: i32 = 1;

/// Process modes
const K_REALTIME: i32 = 0;

/// Media types
const K_AUDIO: i32 = 0;

/// Bus directions
const K_INPUT: i32 = 0;
const K_OUTPUT: i32 = 1;

/// Event types
const K_NOTE_ON_EVENT: u16 = 0;
const K_NOTE_OFF_EVENT: u16 = 1;
const K_DATA_EVENT: u16 = 2;
const K_POLY_PRESSURE_EVENT: u16 = 3;
const K_NOTE_EXPRESSION_VALUE_EVENT: u16 = 4;
#[allow(dead_code)]
const K_NOTE_EXP_VALUE_EVENT: u16 = 4;
#[allow(dead_code)]
const K_NOTE_EXP_TEXT_EVENT: u16 = 5;
#[allow(dead_code)]
const K_CHORD_EVENT: u16 = 6;
#[allow(dead_code)]
const K_SCALE_EVENT: u16 = 7;
#[allow(dead_code)]
const K_LEGACY_MIDI_CC_OUT_EVENT: u16 = 65535;

/// IID for IComponent interface
const IID_ICOMPONENT: [u8; 16] = [
    0xE8, 0x31, 0xFF, 0x31, 0xF2, 0xD5, 0x43, 0x01, 0x92, 0x8E, 0xBB, 0xEE, 0x25, 0x69, 0x78, 0x02,
];

/// IID for IAudioProcessor interface
const IID_IAUDIO_PROCESSOR: [u8; 16] = [
    0x42, 0x04, 0x3F, 0x99, 0xB7, 0xDA, 0x45, 0x3C, 0xA5, 0x69, 0xE7, 0x9D, 0x9A, 0xAE, 0xC3, 0x3D,
];

/// IID for IEditController interface
const IID_IEDIT_CONTROLLER: [u8; 16] = [
    0xDC, 0xD7, 0xBB, 0xE3, 0x77, 0x42, 0x44, 0x8D, 0xA8, 0x74, 0xAA, 0xCC, 0x97, 0x9C, 0x75, 0x9E,
];

/// IID for IEventList interface
const IID_IEVENT_LIST: [u8; 16] = [
    0x3A, 0x2C, 0x4D, 0xC4, 0xF2, 0x57, 0x43, 0xF8, 0x81, 0x98, 0xC9, 0x65, 0xF5, 0xCB, 0xB0, 0xB0,
];

/// IPluginFactory vtable (COM-style)
#[repr(C)]
struct IPluginFactoryVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPluginFactory
    get_factory_info: unsafe extern "system" fn(*mut c_void, *mut PFactoryInfo) -> i32,
    count_classes: unsafe extern "system" fn(*mut c_void) -> i32,
    get_class_info: unsafe extern "system" fn(*mut c_void, i32, *mut PClassInfo) -> i32,
    create_instance: unsafe extern "system" fn(
        *mut c_void,
        *const [u8; 16],
        *const [u8; 16],
        *mut *mut c_void,
    ) -> i32,
}

/// Factory info struct
#[repr(C)]
struct PFactoryInfo {
    vendor: [i8; 64],
    url: [i8; 256],
    email: [i8; 128],
    flags: i32,
}

impl Default for PFactoryInfo {
    fn default() -> Self {
        Self {
            vendor: [0; 64],
            url: [0; 256],
            email: [0; 128],
            flags: 0,
        }
    }
}

/// Class info struct
#[repr(C)]
#[derive(Clone)]
struct PClassInfo {
    cid: [u8; 16],
    cardinality: i32,
    category: [i8; 32],
    name: [i8; 64],
}

impl Default for PClassInfo {
    fn default() -> Self {
        Self {
            cid: [0; 16],
            cardinality: 0,
            category: [0; 32],
            name: [0; 64],
        }
    }
}

/// IPluginBase vtable
#[repr(C)]
#[allow(dead_code)]
struct IPluginBaseVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPluginBase
    initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut c_void) -> i32,
}

/// IComponent vtable (extends IPluginBase)
#[repr(C)]
struct IComponentVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPluginBase
    initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut c_void) -> i32,
    // IComponent
    get_controller_class_id: unsafe extern "system" fn(*mut c_void, *mut [u8; 16]) -> i32,
    set_io_mode: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_bus_count: unsafe extern "system" fn(*mut c_void, i32, i32) -> i32,
    get_bus_info: unsafe extern "system" fn(*mut c_void, i32, i32, i32, *mut c_void) -> i32,
    get_routing_info: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
    activate_bus: unsafe extern "system" fn(*mut c_void, i32, i32, i32, u8) -> i32,
    set_active: unsafe extern "system" fn(*mut c_void, u8) -> i32,
    set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
}

/// IAudioProcessor vtable
#[repr(C)]
struct IAudioProcessorVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IAudioProcessor
    set_bus_arrangements:
        unsafe extern "system" fn(*mut c_void, *mut u64, i32, *mut u64, i32) -> i32,
    get_bus_arrangement: unsafe extern "system" fn(*mut c_void, i32, i32, *mut u64) -> i32,
    can_process_sample_size: unsafe extern "system" fn(*mut c_void, i32) -> i32,
    get_latency_samples: unsafe extern "system" fn(*mut c_void) -> u32,
    setup_processing: unsafe extern "system" fn(*mut c_void, *const ProcessSetup) -> i32,
    set_processing: unsafe extern "system" fn(*mut c_void, u8) -> i32,
    process: unsafe extern "system" fn(*mut c_void, *mut ProcessData) -> i32,
    get_tail_samples: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// IEditController vtable
#[repr(C)]
struct IEditControllerVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPluginBase
    initialize: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    terminate: unsafe extern "system" fn(*mut c_void) -> i32,
    // IEditController
    set_component_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    set_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_state: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_parameter_info: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    get_param_string_by_value: unsafe extern "system" fn(*mut c_void, u32, f64, *mut c_void) -> i32,
    get_param_value_by_string:
        unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut f64) -> i32,
    normalized_param_to_plain: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    plain_param_to_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> f64,
    get_param_normalized: unsafe extern "system" fn(*mut c_void, u32) -> f64,
    set_param_normalized: unsafe extern "system" fn(*mut c_void, u32, f64) -> i32,
    set_component_handler: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    create_view: unsafe extern "system" fn(*mut c_void, *const i8) -> *mut c_void,
}

/// IPlugView vtable (GUI window interface)
#[repr(C)]
struct IPlugViewVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IPlugView
    is_platform_type_supported: unsafe extern "system" fn(*mut c_void, *const i8) -> i32,
    attached: unsafe extern "system" fn(*mut c_void, *mut c_void, *const i8) -> i32,
    removed: unsafe extern "system" fn(*mut c_void) -> i32,
    on_wheel: unsafe extern "system" fn(*mut c_void, f32) -> i32,
    on_key_down: unsafe extern "system" fn(*mut c_void, i16, i16, i16) -> i32,
    on_key_up: unsafe extern "system" fn(*mut c_void, i16, i16, i16) -> i32,
    get_size: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
    on_size: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
    on_focus: unsafe extern "system" fn(*mut c_void, u8) -> i32,
    set_frame: unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32,
    can_resize: unsafe extern "system" fn(*mut c_void) -> i32,
    check_size_constraint: unsafe extern "system" fn(*mut c_void, *mut ViewRect) -> i32,
}

/// ViewRect structure for plugin GUI dimensions
#[repr(C)]
struct ViewRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

/// Process setup structure
#[repr(C)]
struct ProcessSetup {
    process_mode: i32,
    symbolic_sample_size: i32,
    max_samples_per_block: i32,
    sample_rate: f64,
}

/// Audio bus buffers
#[repr(C)]
struct AudioBusBuffers {
    num_channels: i32,
    silence_flags: u64,
    buffers: *mut *mut c_void,
}

/// Process data structure
#[repr(C)]
struct ProcessData {
    process_mode: i32,
    symbolic_sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    input_param_changes: *mut c_void,
    output_param_changes: *mut c_void,
    input_events: *mut c_void,
    output_events: *mut c_void,
    context: *mut ProcessContext,
}

/// ProcessContext state flags
const K_PLAYING: u32 = 1 << 1;
const K_CYCLE_ACTIVE: u32 = 1 << 2;
const K_RECORDING: u32 = 1 << 3;
#[allow(dead_code)]
const K_SYSTEM_TIME_VALID: u32 = 1 << 8;
#[allow(dead_code)]
const K_CONT_TIME_VALID: u32 = 1 << 17;
const K_PROJECT_TIME_MUSIC_VALID: u32 = 1 << 9;
const K_BAR_POSITION_VALID: u32 = 1 << 11;
const K_CYCLE_VALID: u32 = 1 << 12;
const K_TEMPO_VALID: u32 = 1 << 10;
const K_TIME_SIG_VALID: u32 = 1 << 13;
#[allow(dead_code)]
const K_CHORD_VALID: u32 = 1 << 18;
#[allow(dead_code)]
const K_SMPTE_VALID: u32 = 1 << 14;
#[allow(dead_code)]
const K_CLOCK_VALID: u32 = 1 << 15;

/// ProcessContext - timing and musical information
#[repr(C)]
struct ProcessContext {
    /// Transport and validity state flags
    state: u32,
    /// Current sample rate
    sample_rate: f64,
    /// Project time in samples (always valid)
    project_time_samples: i64,
    /// System time in nanoseconds (optional)
    system_time: i64,
    /// Continuous time in samples without loop (optional)
    continuous_time_samples: i64,
    /// Musical position in quarter notes (optional)
    project_time_music: f64,
    /// Last bar start position in quarter notes (optional)
    bar_position_music: f64,
    /// Cycle/loop start in quarter notes (optional)
    cycle_start_music: f64,
    /// Cycle/loop end in quarter notes (optional)
    cycle_end_music: f64,
    /// Tempo in BPM (optional)
    tempo: f64,
    /// Time signature numerator (optional)
    time_sig_numerator: i32,
    /// Time signature denominator (optional)
    time_sig_denominator: i32,
    /// Musical chord info (optional)
    chord: [u8; 12], // Simplified - full Chord struct is complex
    /// SMPTE frame offset (optional)
    smpte_offset_subframes: i32,
    /// Video frame rate (optional)
    frame_rate: i32,
    /// Samples to next MIDI clock (24 ppq)
    samples_to_next_clock: i32,
}

impl Default for ProcessContext {
    fn default() -> Self {
        Self {
            state: 0,
            sample_rate: 44100.0,
            project_time_samples: 0,
            system_time: 0,
            continuous_time_samples: 0,
            project_time_music: 0.0,
            bar_position_music: 0.0,
            cycle_start_music: 0.0,
            cycle_end_music: 0.0,
            tempo: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            chord: [0; 12],
            smpte_offset_subframes: 0,
            frame_rate: 0,
            samples_to_next_clock: 0,
        }
    }
}

impl ProcessContext {
    /// Create from protocol::TransportInfo
    fn from_transport_info(transport: &crate::protocol::TransportInfo, sample_rate: f64) -> Self {
        let mut state = 0u32;

        // Set transport state flags
        if transport.playing {
            state |= K_PLAYING;
        }
        if transport.recording {
            state |= K_RECORDING;
        }
        if transport.cycle_active {
            state |= K_CYCLE_ACTIVE | K_CYCLE_VALID;
        }

        // Set validity flags for fields we populate
        state |=
            K_PROJECT_TIME_MUSIC_VALID | K_BAR_POSITION_VALID | K_TEMPO_VALID | K_TIME_SIG_VALID;

        Self {
            state,
            sample_rate,
            project_time_samples: transport.position_samples,
            system_time: 0,
            continuous_time_samples: transport.position_samples,
            project_time_music: transport.position_quarters,
            bar_position_music: transport.bar_position_quarters,
            cycle_start_music: transport.cycle_start_quarters,
            cycle_end_music: transport.cycle_end_quarters,
            tempo: transport.tempo,
            time_sig_numerator: transport.time_sig_numerator,
            time_sig_denominator: transport.time_sig_denominator,
            chord: [0; 12],
            smpte_offset_subframes: 0,
            frame_rate: 0,
            samples_to_next_clock: 0,
        }
    }
}

/// VST3 Event header (common to all event types)
#[repr(C)]
#[derive(Clone, Copy)]
struct EventHeader {
    bus_index: i32,
    sample_offset: i32,
    ppq_position: f64,
    flags: u16,
    event_type: u16,
}

/// VST3 NoteOn Event
#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOnEvent {
    header: EventHeader,
    channel: i16,
    pitch: i16,
    tuning: f32,
    velocity: f32,
    length: i32,
    note_id: i32,
}

/// VST3 NoteOff Event
#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOffEvent {
    header: EventHeader,
    channel: i16,
    pitch: i16,
    velocity: f32,
    note_id: i32,
    tuning: f32,
}

/// VST3 Data Event (for MIDI CC, pitch bend, etc.)
#[repr(C)]
#[derive(Clone, Copy)]
struct DataEvent {
    header: EventHeader,
    size: u32,
    event_type: u32,
    bytes: [u8; 16], // Up to 16 bytes of MIDI data
}

/// VST3 Poly Pressure Event
#[repr(C)]
#[derive(Clone, Copy)]
struct PolyPressureEvent {
    header: EventHeader,
    channel: i16,
    pitch: i16,
    pressure: f32,
    note_id: i32,
}

/// VST3 Note Expression Value Event (per-note modulation)
#[repr(C)]
#[derive(Clone, Copy)]
struct NoteExpressionValueEvent {
    header: EventHeader,
    /// Note ID (unique identifier for the note)
    note_id: i32,
    /// Expression type ID (0=volume, 1=pan, 2=tuning, 3=vibrato, 4=brightness)
    type_id: u32,
    /// Normalized value (0.0 to 1.0, meaning depends on type_id)
    value: f64,
}

/// Union-like enum for all VST3 event types
#[derive(Clone, Copy)]
enum Vst3Event {
    NoteOn(NoteOnEvent),
    NoteOff(NoteOffEvent),
    Data(DataEvent),
    PolyPressure(PolyPressureEvent),
    NoteExpression(NoteExpressionValueEvent),
}

/// IUnknown vtable
#[repr(C)]
struct IUnknownVtable {
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

/// IEventList vtable
#[repr(C)]
struct IEventListVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IEventList
    get_event_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_event: unsafe extern "system" fn(*mut c_void, i32, *mut c_void) -> i32,
    add_event: unsafe extern "system" fn(*mut c_void, *const c_void) -> i32,
}

/// Our EventList implementation
struct EventList {
    vtable: *const IEventListVtable,
    ref_count: std::sync::atomic::AtomicU32,
    events: Vec<Vst3Event>,
}

impl EventList {
    /// Create an empty event list for initialization only
    fn new_empty() -> Box<Self> {
        let mut event_list = Box::new(EventList {
            vtable: &EVENT_LIST_VTABLE,
            ref_count: std::sync::atomic::AtomicU32::new(1),
            events: Vec::new(),
        });

        event_list.vtable = &EVENT_LIST_VTABLE;
        event_list
    }

    /// Convert VST3 events back to MIDI events
    fn to_midi_events(&self) -> crate::protocol::MidiEventVec {
        self.events
            .iter()
            .filter_map(Self::vst3_to_midi_event)
            .collect()
    }

    /// Extract note expression events from event list
    fn to_note_expression_changes(&self) -> crate::protocol::NoteExpressionChanges {
        let mut changes = crate::protocol::NoteExpressionChanges::new();

        for event in &self.events {
            if let Vst3Event::NoteExpression(e) = event {
                let expression_type = match e.type_id {
                    0 => crate::protocol::NoteExpressionType::Volume,
                    1 => crate::protocol::NoteExpressionType::Pan,
                    2 => crate::protocol::NoteExpressionType::Tuning,
                    3 => crate::protocol::NoteExpressionType::Vibrato,
                    4 => crate::protocol::NoteExpressionType::Brightness,
                    _ => continue, // Unknown type, skip
                };

                changes.add_change(crate::protocol::NoteExpressionValue {
                    sample_offset: e.header.sample_offset,
                    note_id: e.note_id,
                    expression_type,
                    value: e.value,
                });
            }
        }

        changes
    }

    /// Update existing event list from MIDI events (RT-safe: reuses allocation)
    fn update_from_midi(&mut self, midi_events: &[MidiEvent]) {
        self.events.clear();
        self.events.extend(
            midi_events
                .iter()
                .filter_map(Self::midi_to_vst3_event)
        );
    }

    /// Update from MIDI and note expression (RT-safe: reuses allocation)
    fn update_from_midi_and_expression(
        &mut self,
        midi_events: &[MidiEvent],
        note_expression: &crate::protocol::NoteExpressionChanges,
    ) {
        self.events.clear();

        // Add MIDI events
        self.events.extend(
            midi_events
                .iter()
                .filter_map(Self::midi_to_vst3_event)
        );

        // Add note expression events
        for expr in &note_expression.changes {
            self.events.push(Self::note_expression_to_vst3_event(expr));
        }

        // Sort by sample offset for proper event ordering
        self.events.sort_by_key(|event| match event {
            Vst3Event::NoteOn(e) => e.header.sample_offset,
            Vst3Event::NoteOff(e) => e.header.sample_offset,
            Vst3Event::Data(e) => e.header.sample_offset,
            Vst3Event::PolyPressure(e) => e.header.sample_offset,
            Vst3Event::NoteExpression(e) => e.header.sample_offset,
        });
    }

    /// Clear events (RT-safe: keeps allocation)
    fn clear(&mut self) {
        self.events.clear();
    }

    /// Convert VST3 Event to Tutti MidiEvent
    fn vst3_to_midi_event(event: &Vst3Event) -> Option<MidiEvent> {
        match event {
            Vst3Event::NoteOn(e) => Some(MidiEvent {
                frame_offset: e.header.sample_offset as usize,
                channel: Channel::from_u8(e.channel as u8),
                msg: ChannelVoiceMsg::NoteOn {
                    note: e.pitch as u8,
                    velocity: (e.velocity * 127.0) as u8,
                },
            }),
            Vst3Event::NoteOff(e) => Some(MidiEvent {
                frame_offset: e.header.sample_offset as usize,
                channel: Channel::from_u8(e.channel as u8),
                msg: ChannelVoiceMsg::NoteOff {
                    note: e.pitch as u8,
                    velocity: (e.velocity * 127.0) as u8,
                },
            }),
            Vst3Event::PolyPressure(e) => Some(MidiEvent {
                frame_offset: e.header.sample_offset as usize,
                channel: Channel::from_u8(e.channel as u8),
                msg: ChannelVoiceMsg::PolyPressure {
                    note: e.pitch as u8,
                    pressure: (e.pressure * 127.0) as u8,
                },
            }),
            Vst3Event::Data(e) => {
                // Parse MIDI bytes from DataEvent
                if e.size < 3 {
                    return None;
                }
                let status = e.bytes[0];
                let channel = Channel::from_u8(status & 0x0F);
                let msg_type = status & 0xF0;

                match msg_type {
                    0xB0 => {
                        // Control Change
                        Some(MidiEvent {
                            frame_offset: e.header.sample_offset as usize,
                            channel,
                            msg: ChannelVoiceMsg::ControlChange {
                                control: ControlChange::CC {
                                    control: e.bytes[1],
                                    value: e.bytes[2],
                                },
                            },
                        })
                    }
                    0xC0 => {
                        // Program Change
                        Some(MidiEvent {
                            frame_offset: e.header.sample_offset as usize,
                            channel,
                            msg: ChannelVoiceMsg::ProgramChange {
                                program: e.bytes[1],
                            },
                        })
                    }
                    0xE0 => {
                        // Pitch Bend
                        let bend = ((e.bytes[2] as u16) << 7) | (e.bytes[1] as u16);
                        Some(MidiEvent {
                            frame_offset: e.header.sample_offset as usize,
                            channel,
                            msg: ChannelVoiceMsg::PitchBend { bend },
                        })
                    }
                    _ => None,
                }
            }
            Vst3Event::NoteExpression(_) => {
                // Note expression is not MIDI, skip it in MIDI conversion
                None
            }
        }
    }

    /// Convert Tutti MidiEvent to VST3 Event
    fn midi_to_vst3_event(event: &MidiEvent) -> Option<Vst3Event> {
        let channel_num = event.channel as i16;
        let sample_offset = event.frame_offset as i32;

        let header = EventHeader {
            bus_index: 0,
            sample_offset,
            ppq_position: 0.0,
            flags: 0,
            event_type: 0, // Will be set below
        };

        match event.msg {
            ChannelVoiceMsg::NoteOn { note, velocity } => {
                let mut header = header;
                header.event_type = K_NOTE_ON_EVENT;
                Some(Vst3Event::NoteOn(NoteOnEvent {
                    header,
                    channel: channel_num,
                    pitch: note as i16,
                    tuning: 0.0,
                    velocity: (velocity as f32) / 127.0,
                    length: 0,
                    note_id: -1,
                }))
            }
            ChannelVoiceMsg::NoteOff { note, velocity } => {
                let mut header = header;
                header.event_type = K_NOTE_OFF_EVENT;
                Some(Vst3Event::NoteOff(NoteOffEvent {
                    header,
                    channel: channel_num,
                    pitch: note as i16,
                    velocity: (velocity as f32) / 127.0,
                    note_id: -1,
                    tuning: 0.0,
                }))
            }
            ChannelVoiceMsg::PolyPressure { note, pressure } => {
                let mut header = header;
                header.event_type = K_POLY_PRESSURE_EVENT;
                Some(Vst3Event::PolyPressure(PolyPressureEvent {
                    header,
                    channel: channel_num,
                    pitch: note as i16,
                    pressure: (pressure as f32) / 127.0,
                    note_id: -1,
                }))
            }
            ChannelVoiceMsg::ControlChange { control } => {
                // Convert to MIDI bytes for Data event
                let channel_byte = event.channel as u8;
                let (cc, value) = match control {
                    ControlChange::CC { control, value } => (control, value),
                    ControlChange::CCHighRes {
                        control1, value, ..
                    } => (control1, (value >> 7) as u8),
                    _ => return None,
                };

                let mut header = header;
                header.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xB0 | channel_byte;
                bytes[1] = cc;
                bytes[2] = value;

                Some(Vst3Event::Data(DataEvent {
                    header,
                    size: 3,
                    event_type: 0, // Legacy MIDI
                    bytes,
                }))
            }
            ChannelVoiceMsg::ProgramChange { program } => {
                let mut header = header;
                header.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xC0 | (event.channel as u8);
                bytes[1] = program;

                Some(Vst3Event::Data(DataEvent {
                    header,
                    size: 2,
                    event_type: 0,
                    bytes,
                }))
            }
            ChannelVoiceMsg::ChannelPressure { pressure } => {
                let mut header = header;
                header.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xD0 | (event.channel as u8);
                bytes[1] = pressure;

                Some(Vst3Event::Data(DataEvent {
                    header,
                    size: 2,
                    event_type: 0,
                    bytes,
                }))
            }
            ChannelVoiceMsg::PitchBend { bend } => {
                let mut header = header;
                header.event_type = K_DATA_EVENT;
                let mut bytes = [0u8; 16];
                bytes[0] = 0xE0 | (event.channel as u8);
                bytes[1] = (bend & 0x7F) as u8;
                bytes[2] = ((bend >> 7) & 0x7F) as u8;

                Some(Vst3Event::Data(DataEvent {
                    header,
                    size: 3,
                    event_type: 0,
                    bytes,
                }))
            }
            _ => None,
        }
    }

    /// Convert protocol note expression to VST3 event
    fn note_expression_to_vst3_event(expr: &crate::protocol::NoteExpressionValue) -> Vst3Event {
        let header = EventHeader {
            bus_index: 0,
            sample_offset: expr.sample_offset,
            ppq_position: 0.0,
            flags: 0,
            event_type: K_NOTE_EXPRESSION_VALUE_EVENT,
        };

        // Map protocol expression type to VST3 type ID
        let type_id = match expr.expression_type {
            crate::protocol::NoteExpressionType::Volume => 0,
            crate::protocol::NoteExpressionType::Pan => 1,
            crate::protocol::NoteExpressionType::Tuning => 2,
            crate::protocol::NoteExpressionType::Vibrato => 3,
            crate::protocol::NoteExpressionType::Brightness => 4,
        };

        Vst3Event::NoteExpression(NoteExpressionValueEvent {
            header,
            note_id: expr.note_id,
            type_id,
            value: expr.value,
        })
    }
}

// EventList vtable implementation
static EVENT_LIST_VTABLE: IEventListVtable = IEventListVtable {
    query_interface: event_list_query_interface,
    add_ref: event_list_add_ref,
    release: event_list_release,
    get_event_count: event_list_get_event_count,
    get_event: event_list_get_event,
    add_event: event_list_add_event,
};

unsafe extern "system" fn event_list_query_interface(
    this: *mut c_void,
    iid: *const [u8; 16],
    obj: *mut *mut c_void,
) -> i32 {
    if (*iid) == IID_IEVENT_LIST || (*iid) == [0; 16] {
        *obj = this;
        event_list_add_ref(this);
        K_RESULT_OK
    } else {
        *obj = std::ptr::null_mut();
        -1
    }
}

unsafe extern "system" fn event_list_add_ref(this: *mut c_void) -> u32 {
    let event_list = &*(this as *const EventList);
    event_list
        .ref_count
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1
}

unsafe extern "system" fn event_list_release(this: *mut c_void) -> u32 {
    let event_list = &*(this as *const EventList);
    let count = event_list
        .ref_count
        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
        - 1;
    if count == 0 {
        let _ = Box::from_raw(this as *mut EventList);
    }
    count
}

unsafe extern "system" fn event_list_get_event_count(this: *mut c_void) -> i32 {
    let event_list = &*(this as *const EventList);
    event_list.events.len() as i32
}

unsafe extern "system" fn event_list_get_event(
    this: *mut c_void,
    index: i32,
    event: *mut c_void,
) -> i32 {
    let event_list = &*(this as *const EventList);
    if index < 0 || index >= event_list.events.len() as i32 {
        return -1;
    }

    // Copy the event data to the output pointer
    match &event_list.events[index as usize] {
        Vst3Event::NoteOn(e) => {
            std::ptr::copy_nonoverlapping(
                e as *const NoteOnEvent as *const u8,
                event as *mut u8,
                std::mem::size_of::<NoteOnEvent>(),
            );
        }
        Vst3Event::NoteOff(e) => {
            std::ptr::copy_nonoverlapping(
                e as *const NoteOffEvent as *const u8,
                event as *mut u8,
                std::mem::size_of::<NoteOffEvent>(),
            );
        }
        Vst3Event::Data(e) => {
            std::ptr::copy_nonoverlapping(
                e as *const DataEvent as *const u8,
                event as *mut u8,
                std::mem::size_of::<DataEvent>(),
            );
        }
        Vst3Event::PolyPressure(e) => {
            std::ptr::copy_nonoverlapping(
                e as *const PolyPressureEvent as *const u8,
                event as *mut u8,
                std::mem::size_of::<PolyPressureEvent>(),
            );
        }
        Vst3Event::NoteExpression(e) => {
            std::ptr::copy_nonoverlapping(
                e as *const NoteExpressionValueEvent as *const u8,
                event as *mut u8,
                std::mem::size_of::<NoteExpressionValueEvent>(),
            );
        }
    }

    K_RESULT_OK
}

unsafe extern "system" fn event_list_add_event(_this: *mut c_void, _event: *const c_void) -> i32 {
    // Not implemented - we're read-only for input events
    -1
}

// ============================================================================
// Parameter Automation Support (IParamValueQueue, IParameterChanges)
// ============================================================================

/// IParamValueQueue vtable
#[repr(C)]
struct IParamValueQueueVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IParamValueQueue
    get_parameter_id: unsafe extern "system" fn(*mut c_void) -> u32,
    get_point_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_point: unsafe extern "system" fn(*mut c_void, i32, *mut i32, *mut f64) -> i32,
    add_point: unsafe extern "system" fn(*mut c_void, i32, f64, *mut i32) -> i32,
}

/// Our ParamValueQueue implementation
struct ParamValueQueue {
    vtable: *const IParamValueQueueVtable,
    ref_count: std::sync::atomic::AtomicU32,
    param_id: u32,
    points: Vec<crate::protocol::ParameterPoint>,
}

impl ParamValueQueue {
    /// Create from protocol::ParameterQueue
    fn from_protocol(queue: &crate::protocol::ParameterQueue) -> Box<Self> {
        let mut param_queue = Box::new(ParamValueQueue {
            vtable: &PARAM_VALUE_QUEUE_VTABLE,
            ref_count: std::sync::atomic::AtomicU32::new(1),
            param_id: queue.param_id,
            points: queue.points.clone(),
        });
        param_queue.vtable = &PARAM_VALUE_QUEUE_VTABLE;
        param_queue
    }

    /// Create empty queue for output
    fn new_empty(param_id: u32) -> Box<Self> {
        let mut param_queue = Box::new(ParamValueQueue {
            vtable: &PARAM_VALUE_QUEUE_VTABLE,
            ref_count: std::sync::atomic::AtomicU32::new(1),
            param_id,
            points: Vec::new(),
        });
        param_queue.vtable = &PARAM_VALUE_QUEUE_VTABLE;
        param_queue
    }

    /// Convert to protocol::ParameterQueue
    fn to_protocol(&self) -> crate::protocol::ParameterQueue {
        crate::protocol::ParameterQueue {
            param_id: self.param_id,
            points: self.points.clone(),
        }
    }
}

static PARAM_VALUE_QUEUE_VTABLE: IParamValueQueueVtable = IParamValueQueueVtable {
    query_interface: param_queue_query_interface,
    add_ref: param_queue_add_ref,
    release: param_queue_release,
    get_parameter_id: param_queue_get_parameter_id,
    get_point_count: param_queue_get_point_count,
    get_point: param_queue_get_point,
    add_point: param_queue_add_point,
};

unsafe extern "system" fn param_queue_query_interface(
    this: *mut c_void,
    _iid: *const [u8; 16],
    obj: *mut *mut c_void,
) -> i32 {
    *obj = this;
    param_queue_add_ref(this);
    K_RESULT_OK
}

unsafe extern "system" fn param_queue_add_ref(this: *mut c_void) -> u32 {
    let queue = &*(this as *const ParamValueQueue);
    queue
        .ref_count
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1
}

unsafe extern "system" fn param_queue_release(this: *mut c_void) -> u32 {
    let queue = &*(this as *const ParamValueQueue);
    let count = queue
        .ref_count
        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
        - 1;
    if count == 0 {
        let _ = Box::from_raw(this as *mut ParamValueQueue);
    }
    count
}

unsafe extern "system" fn param_queue_get_parameter_id(this: *mut c_void) -> u32 {
    let queue = &*(this as *const ParamValueQueue);
    queue.param_id
}

unsafe extern "system" fn param_queue_get_point_count(this: *mut c_void) -> i32 {
    let queue = &*(this as *const ParamValueQueue);
    queue.points.len() as i32
}

unsafe extern "system" fn param_queue_get_point(
    this: *mut c_void,
    index: i32,
    sample_offset: *mut i32,
    value: *mut f64,
) -> i32 {
    let queue = &*(this as *const ParamValueQueue);
    if index < 0 || index >= queue.points.len() as i32 {
        return -1;
    }

    let point = &queue.points[index as usize];
    *sample_offset = point.sample_offset;
    *value = point.value;
    K_RESULT_OK
}

unsafe extern "system" fn param_queue_add_point(
    this: *mut c_void,
    sample_offset: i32,
    value: f64,
    index: *mut i32,
) -> i32 {
    let queue = &mut *(this as *mut ParamValueQueue);
    queue.points.push(crate::protocol::ParameterPoint {
        sample_offset,
        value,
    });
    *index = (queue.points.len() - 1) as i32;
    K_RESULT_OK
}

/// IParameterChanges vtable
#[repr(C)]
struct IParameterChangesVtable {
    // IUnknown
    query_interface:
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> i32,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
    // IParameterChanges
    get_parameter_count: unsafe extern "system" fn(*mut c_void) -> i32,
    get_parameter_data: unsafe extern "system" fn(*mut c_void, i32) -> *mut c_void,
    add_parameter_data: unsafe extern "system" fn(*mut c_void, *const u32, *mut i32) -> *mut c_void,
}

/// Our ParameterChanges implementation
struct ParameterChanges {
    vtable: *const IParameterChangesVtable,
    ref_count: std::sync::atomic::AtomicU32,
    #[allow(clippy::vec_box)] // Box needed for stable pointers in COM interface
    queues: Vec<Box<ParamValueQueue>>,
}

impl ParameterChanges {
    /// Create from protocol::ParameterChanges
    fn from_protocol(changes: &crate::protocol::ParameterChanges) -> Box<Self> {
        let queues: Vec<Box<ParamValueQueue>> = changes
            .queues
            .iter()
            .map(ParamValueQueue::from_protocol)
            .collect();

        let mut param_changes = Box::new(ParameterChanges {
            vtable: &PARAMETER_CHANGES_VTABLE,
            ref_count: std::sync::atomic::AtomicU32::new(1),
            queues,
        });
        param_changes.vtable = &PARAMETER_CHANGES_VTABLE;
        param_changes
    }

    /// Create empty for output
    fn new_empty() -> Box<Self> {
        let mut param_changes = Box::new(ParameterChanges {
            vtable: &PARAMETER_CHANGES_VTABLE,
            ref_count: std::sync::atomic::AtomicU32::new(1),
            queues: Vec::new(),
        });
        param_changes.vtable = &PARAMETER_CHANGES_VTABLE;
        param_changes
    }

    /// Convert to protocol::ParameterChanges
    fn to_protocol(&self) -> crate::protocol::ParameterChanges {
        let queues = self.queues.iter().map(|q| q.to_protocol()).collect();
        crate::protocol::ParameterChanges { queues }
    }
}

static PARAMETER_CHANGES_VTABLE: IParameterChangesVtable = IParameterChangesVtable {
    query_interface: param_changes_query_interface,
    add_ref: param_changes_add_ref,
    release: param_changes_release,
    get_parameter_count: param_changes_get_parameter_count,
    get_parameter_data: param_changes_get_parameter_data,
    add_parameter_data: param_changes_add_parameter_data,
};

unsafe extern "system" fn param_changes_query_interface(
    this: *mut c_void,
    _iid: *const [u8; 16],
    obj: *mut *mut c_void,
) -> i32 {
    *obj = this;
    param_changes_add_ref(this);
    K_RESULT_OK
}

unsafe extern "system" fn param_changes_add_ref(this: *mut c_void) -> u32 {
    let changes = &*(this as *const ParameterChanges);
    changes
        .ref_count
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1
}

unsafe extern "system" fn param_changes_release(this: *mut c_void) -> u32 {
    let changes = &*(this as *const ParameterChanges);
    let count = changes
        .ref_count
        .fetch_sub(1, std::sync::atomic::Ordering::SeqCst)
        - 1;
    if count == 0 {
        let _ = Box::from_raw(this as *mut ParameterChanges);
    }
    count
}

unsafe extern "system" fn param_changes_get_parameter_count(this: *mut c_void) -> i32 {
    let changes = &*(this as *const ParameterChanges);
    changes.queues.len() as i32
}

unsafe extern "system" fn param_changes_get_parameter_data(
    this: *mut c_void,
    index: i32,
) -> *mut c_void {
    let changes = &*(this as *const ParameterChanges);
    if index < 0 || index >= changes.queues.len() as i32 {
        return std::ptr::null_mut();
    }

    &*changes.queues[index as usize] as *const ParamValueQueue as *mut c_void
}

unsafe extern "system" fn param_changes_add_parameter_data(
    this: *mut c_void,
    param_id: *const u32,
    index: *mut i32,
) -> *mut c_void {
    let changes = &mut *(this as *mut ParameterChanges);
    let new_queue = ParamValueQueue::new_empty(*param_id);
    let queue_ptr = &*new_queue as *const ParamValueQueue as *mut c_void;
    changes.queues.push(new_queue);
    *index = (changes.queues.len() - 1) as i32;
    queue_ptr
}

/// VST3 library wrapper
struct Vst3Library {
    _library: Library,
    factory: *mut c_void,
    vtable: *const IPluginFactoryVtable,
}

unsafe impl Send for Vst3Library {}
unsafe impl Sync for Vst3Library {}

impl Vst3Library {
    /// Load a VST3 library from the bundle path
    fn load(bundle_path: &Path) -> Result<Self> {
        // Locate the actual library file within the bundle
        let lib_path = Self::find_library_path(bundle_path)?;

        // Load the shared library
        let library = unsafe {
            Library::new(&lib_path).map_err(|e| BridgeError::LoadFailed {
                path: lib_path.clone(),
                stage: LoadStage::Opening,
                reason: e.to_string(),
            })?
        };

        // Get the factory function
        let get_factory: libloading::Symbol<GetPluginFactoryFn> = unsafe {
            library.get(b"GetPluginFactory\0").map_err(|e| BridgeError::LoadFailed {
                path: lib_path.clone(),
                stage: LoadStage::Factory,
                reason: format!("Missing GetPluginFactory symbol: {}", e),
            })?
        };

        // Call the factory function
        let factory = unsafe { get_factory() };
        if factory.is_null() {
            return Err(BridgeError::LoadFailed {
                path: lib_path,
                stage: LoadStage::Factory,
                reason: "GetPluginFactory returned null".to_string(),
            });
        }

        // Get vtable from the factory object (first pointer is vtable)
        let vtable = unsafe { *(factory as *const *const IPluginFactoryVtable) };

        Ok(Self {
            _library: library,
            factory,
            vtable,
        })
    }

    /// Find the actual library file within a .vst3 bundle
    fn find_library_path(bundle_path: &Path) -> Result<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let lib_name = bundle_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| BridgeError::LoadFailed {
                    path: bundle_path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: "Failed to extract plugin name from bundle path".to_string(),
                })?;

            let lib_path = bundle_path.join("Contents").join("MacOS").join(lib_name);
            if lib_path.exists() {
                return Ok(lib_path);
            }
        }

        #[cfg(target_os = "linux")]
        {
            let lib_name = bundle_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| BridgeError::LoadFailed {
                    path: bundle_path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: "Failed to extract plugin name from bundle path".to_string(),
                })?;

            let lib_path = bundle_path
                .join("Contents")
                .join("x86_64-linux")
                .join(format!("{}.so", lib_name));
            if lib_path.exists() {
                return Ok(lib_path);
            }
        }

        #[cfg(target_os = "windows")]
        {
            let lib_name = bundle_path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| BridgeError::LoadFailed {
                    path: bundle_path.to_path_buf(),
                    stage: LoadStage::Opening,
                    reason: "Failed to extract plugin name from bundle path".to_string(),
                })?;

            let lib_path = bundle_path
                .join("Contents")
                .join("x86_64-win")
                .join(format!("{}.vst3", lib_name));
            if lib_path.exists() {
                return Ok(lib_path);
            }
        }

        // If bundle is a direct library file, use it directly
        if bundle_path.is_file() {
            return Ok(bundle_path.to_path_buf());
        }

        Err(BridgeError::LoadFailed {
            path: bundle_path.to_path_buf(),
            stage: LoadStage::Opening,
            reason: "Could not find library in VST3 bundle".to_string(),
        })
    }

    /// Get factory info
    fn get_factory_info(&self) -> Option<PFactoryInfo> {
        let mut info = PFactoryInfo::default();
        let result = unsafe { ((*self.vtable).get_factory_info)(self.factory, &mut info) };
        if result == K_RESULT_OK {
            Some(info)
        } else {
            None
        }
    }

    /// Get number of plugin classes
    fn count_classes(&self) -> i32 {
        unsafe { ((*self.vtable).count_classes)(self.factory) }
    }

    /// Get class info at index
    fn get_class_info(&self, index: i32) -> Result<PClassInfo> {
        let mut info = PClassInfo::default();
        let result = unsafe { ((*self.vtable).get_class_info)(self.factory, index, &mut info) };
        if result == K_RESULT_OK {
            Ok(info)
        } else {
            Err(BridgeError::PluginError {
                stage: LoadStage::Factory,
                code: result,
            })
        }
    }

    /// Create an instance of a plugin class
    fn create_instance(&self, cid: &[u8; 16], iid: &[u8; 16]) -> Result<*mut c_void> {
        let mut obj: *mut c_void = std::ptr::null_mut();
        let result = unsafe { ((*self.vtable).create_instance)(self.factory, cid, iid, &mut obj) };
        if result == K_RESULT_OK && !obj.is_null() {
            Ok(obj)
        } else {
            Err(BridgeError::PluginError {
                stage: LoadStage::Instantiation,
                code: result,
            })
        }
    }

    /// Helper function to convert null-terminated i8 array to String
    fn c_str_to_string(bytes: &[i8]) -> String {
        let bytes: Vec<u8> = bytes
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        String::from_utf8_lossy(&bytes).to_string()
    }
}

impl Drop for Vst3Library {
    fn drop(&mut self) {
        // Release the factory
        unsafe {
            ((*self.vtable).release)(self.factory);
        }
    }
}

/// VST3 plugin instance wrapper
pub struct Vst3Instance {
    _library: Arc<Vst3Library>,

    /// IComponent pointer
    component: *mut c_void,
    component_vtable: *const IComponentVtable,

    /// IAudioProcessor pointer
    processor: *mut c_void,
    processor_vtable: *const IAudioProcessorVtable,

    /// IEditController pointer (optional)
    controller: Option<*mut c_void>,
    controller_vtable: Option<*const IEditControllerVtable>,

    /// IPlugView pointer (optional, for GUI)
    view: Option<*mut c_void>,
    view_vtable: Option<*const IPlugViewVtable>,

    metadata: PluginMetadata,
    sample_rate: f32,
    is_active: bool,
    block_size: usize,

    /// Parameter ID to index mapping (unused for now, but reserved for future use)
    #[allow(dead_code)]
    parameter_map: HashMap<String, u32>,

    /// Input/output buffer pointers for processing (f32)
    input_buffer_ptrs: Vec<*mut f32>,
    output_buffer_ptrs: Vec<*mut f32>,

    /// Input/output buffer pointers for f64 processing
    input_buffer_ptrs_f64: Vec<*mut f64>,
    output_buffer_ptrs_f64: Vec<*mut f64>,

    /// Negotiated sample format for this plugin instance
    sample_format: crate::protocol::SampleFormat,

    /// Editor size (cached from last open)
    editor_size: (u32, u32),

    // RT-safe event list pool (reused across process calls)
    /// Reusable event list for input events
    input_event_list: Option<Box<EventList>>,
    /// Reusable event list for output events
    output_event_list: Option<Box<EventList>>,
}

unsafe impl Send for Vst3Instance {}
unsafe impl Sync for Vst3Instance {}

impl Vst3Instance {
    /// Load a VST3 plugin from path
    pub fn load(path: &Path, sample_rate: f32) -> Result<Self> {
        if !path.exists() {
            return Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Scanning,
                reason: "Plugin file not found".to_string(),
            });
        }

        let library = Arc::new(Vst3Library::load(path)?);
        let count = library.count_classes();

        if count == 0 {
            return Err(BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Factory,
                reason: "VST3 factory contains no classes".to_string(),
            });
        }

        // Get factory info for vendor name
        let factory_info = library.get_factory_info();
        let vendor = factory_info
            .as_ref()
            .map(|info| Vst3Library::c_str_to_string(&info.vendor))
            .unwrap_or_default();

        // Find first audio processor class
        let (class_info, name) = (0..count)
            .find_map(|i| {
                let info = library.get_class_info(i).ok()?;
                let category = Vst3Library::c_str_to_string(&info.category);
                if category.contains("Audio") {
                    let name = Vst3Library::c_str_to_string(&info.name);
                    Some((info, name))
                } else {
                    None
                }
            })
            .ok_or_else(|| BridgeError::LoadFailed {
                path: path.to_path_buf(),
                stage: LoadStage::Factory,
                reason: "No audio processor classes found in VST3".to_string(),
            })?;

        // Create IComponent instance
        let component = library.create_instance(&class_info.cid, &IID_ICOMPONENT)?;

        // Query IAudioProcessor interface
        let processor = {
            let vtable = unsafe { *(component as *const *const IUnknownVtable) };
            let mut proc_ptr: *mut c_void = std::ptr::null_mut();
            let result = unsafe {
                ((*vtable).query_interface)(component, &IID_IAUDIO_PROCESSOR, &mut proc_ptr)
            };
            if result == K_RESULT_OK && !proc_ptr.is_null() {
                proc_ptr
            } else {
                return Err(BridgeError::LoadFailed {
                    path: path.to_path_buf(),
                    stage: LoadStage::Instantiation,
                    reason: "VST3 plugin does not support IAudioProcessor".to_string(),
                });
            }
        };

        // Query IEditController interface (optional)
        let controller = {
            let vtable = unsafe { *(component as *const *const IUnknownVtable) };
            let mut ctrl_ptr: *mut c_void = std::ptr::null_mut();
            let result = unsafe {
                ((*vtable).query_interface)(component, &IID_IEDIT_CONTROLLER, &mut ctrl_ptr)
            };
            if result == K_RESULT_OK && !ctrl_ptr.is_null() {
                Some(ctrl_ptr)
            } else {
                None
            }
        };

        let component_vtable = unsafe { *(component as *const *const IComponentVtable) };
        let processor_vtable = unsafe { *(processor as *const *const IAudioProcessorVtable) };
        let controller_vtable =
            controller.map(|c| unsafe { *(c as *const *const IEditControllerVtable) });

        // Generate unique ID from class ID
        let unique_id = format!(
            "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}",
            class_info.cid[0], class_info.cid[1], class_info.cid[2], class_info.cid[3],
            class_info.cid[4], class_info.cid[5], class_info.cid[6], class_info.cid[7],
            class_info.cid[8], class_info.cid[9], class_info.cid[10], class_info.cid[11],
            class_info.cid[12], class_info.cid[13], class_info.cid[14], class_info.cid[15]
        );

        // Query f64 support via canProcessSampleSize
        let supports_f64 = {
            let vtable = unsafe { &*processor_vtable };
            let result = unsafe { (vtable.can_process_sample_size)(processor, K_SAMPLE_64) };
            result == K_RESULT_OK
        };

        // Build metadata (assume stereo for now, can query later)
        let metadata = PluginMetadata::new(format!("vst3.{}", unique_id), name.clone())
            .author(vendor.clone())
            .version("1.0.0".to_string())
            .audio_io(2, 2)
            .midi(true)
            .f64_support(supports_f64);

        let mut instance = Self {
            _library: library,
            component,
            component_vtable,
            processor,
            processor_vtable,
            controller,
            controller_vtable,
            view: None,
            view_vtable: None,
            metadata,
            sample_rate,
            is_active: false,
            block_size: 512,
            parameter_map: HashMap::new(),
            input_buffer_ptrs: vec![std::ptr::null_mut(); 2],
            output_buffer_ptrs: vec![std::ptr::null_mut(); 2],
            input_buffer_ptrs_f64: vec![std::ptr::null_mut(); 2],
            output_buffer_ptrs_f64: vec![std::ptr::null_mut(); 2],
            sample_format: crate::protocol::SampleFormat::Float32,
            editor_size: (800, 600), // Default size
            // Initialize event list pool for RT-safe reuse
            input_event_list: Some(EventList::new_empty()),
            output_event_list: Some(EventList::new_empty()),
        };

        // Initialize the plugin
        instance.initialize()?;

        Ok(instance)
    }

    /// Initialize the plugin
    fn initialize(&mut self) -> Result<()> {
        // Initialize component
        let result =
            unsafe { ((*self.component_vtable).initialize)(self.component, std::ptr::null_mut()) };

        if result != K_RESULT_OK && result != K_RESULT_TRUE {
            return Err(BridgeError::PluginError {
                stage: LoadStage::Initialization,
                code: result,
            });
        }

        // Initialize controller if separate
        if let (Some(ctrl), Some(vtable)) = (self.controller, self.controller_vtable) {
            let result = unsafe { ((*vtable).initialize)(ctrl, std::ptr::null_mut()) };
            if result != K_RESULT_OK && result != K_RESULT_TRUE {}
        }

        // Setup processing with negotiated sample format
        let symbolic_sample_size = match self.sample_format {
            crate::protocol::SampleFormat::Float32 => K_SAMPLE_32,
            crate::protocol::SampleFormat::Float64 => K_SAMPLE_64,
        };

        let setup = ProcessSetup {
            process_mode: K_REALTIME,
            symbolic_sample_size,
            max_samples_per_block: self.block_size as i32,
            sample_rate: self.sample_rate as f64,
        };

        let result = unsafe { ((*self.processor_vtable).setup_processing)(self.processor, &setup) };

        if result != K_RESULT_OK && result != K_RESULT_TRUE {
            return Err(BridgeError::PluginError {
                stage: LoadStage::Setup,
                code: result,
            });
        }

        // Activate buses
        self.activate_buses()?;

        // Set active state
        let result = unsafe { ((*self.processor_vtable).set_processing)(self.processor, 1) };
        if result != K_RESULT_OK && result != K_RESULT_TRUE {
            return Err(BridgeError::PluginError {
                stage: LoadStage::Activation,
                code: result,
            });
        }

        let result = unsafe { ((*self.component_vtable).set_active)(self.component, 1) };
        if result != K_RESULT_OK && result != K_RESULT_TRUE {
            return Err(BridgeError::PluginError {
                stage: LoadStage::Activation,
                code: result,
            });
        }

        self.is_active = true;
        Ok(())
    }

    /// Activate audio buses
    fn activate_buses(&mut self) -> Result<()> {
        // Activate input buses
        let num_input_buses =
            unsafe { ((*self.component_vtable).get_bus_count)(self.component, K_AUDIO, K_INPUT) };

        for i in 0..num_input_buses {
            let _ = unsafe {
                ((*self.component_vtable).activate_bus)(self.component, K_AUDIO, K_INPUT, i, 1)
            };
        }

        // Activate output buses
        let num_output_buses =
            unsafe { ((*self.component_vtable).get_bus_count)(self.component, K_AUDIO, K_OUTPUT) };

        for i in 0..num_output_buses {
            let _ = unsafe {
                ((*self.component_vtable).activate_bus)(self.component, K_AUDIO, K_OUTPUT, i, 1)
            };
        }

        Ok(())
    }

    /// Get plugin metadata
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    /// Check if this plugin supports 64-bit (f64) audio processing
    pub fn can_process_f64(&self) -> bool {
        self.metadata.supports_f64
    }

    /// Set the sample format for processing.
    ///
    /// Must be called before `initialize()` if switching to f64.
    /// If the plugin doesn't support f64, this will return an error.
    pub fn set_sample_format(&mut self, format: crate::protocol::SampleFormat) -> Result<()> {
        if format == crate::protocol::SampleFormat::Float64 && !self.metadata.supports_f64 {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Setup,
                reason: "Plugin does not support 64-bit audio processing".to_string(),
            });
        }
        self.sample_format = format;
        Ok(())
    }

    /// Process audio through the plugin
    pub fn process(&mut self, buffer: &mut AudioBuffer) {
        if !self.is_active {
            return;
        }

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return;
        }

        // Update buffer pointers
        for (i, input_slice) in buffer.inputs.iter().enumerate() {
            if i < self.input_buffer_ptrs.len() {
                self.input_buffer_ptrs[i] = input_slice.as_ptr() as *mut f32;
            }
        }

        for (i, output_slice) in buffer.outputs.iter_mut().enumerate() {
            if i < self.output_buffer_ptrs.len() {
                self.output_buffer_ptrs[i] = output_slice.as_mut_ptr();
            }
        }

        // Create audio bus buffers
        let mut input_bus = AudioBusBuffers {
            num_channels: buffer.inputs.len() as i32,
            silence_flags: 0,
            buffers: self.input_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        let mut output_bus = AudioBusBuffers {
            num_channels: buffer.outputs.len() as i32,
            silence_flags: 0,
            buffers: self.output_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        // Create process data
        let mut process_data = ProcessData {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_32,
            num_samples: num_samples as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            input_param_changes: std::ptr::null_mut(),
            output_param_changes: std::ptr::null_mut(),
            input_events: std::ptr::null_mut(),
            output_events: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
        };

        // Process through VST3
        let result =
            unsafe { ((*self.processor_vtable).process)(self.processor, &mut process_data) };

        if result != K_RESULT_OK {}
    }

    /// Process audio with MIDI events through the plugin
    pub fn process_with_midi(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
    ) -> crate::protocol::MidiEventVec {
        if !self.is_active {
            return smallvec::SmallVec::new();
        }

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return smallvec::SmallVec::new();
        }

        // Update buffer pointers
        for (i, input_slice) in buffer.inputs.iter().enumerate() {
            if i < self.input_buffer_ptrs.len() {
                self.input_buffer_ptrs[i] = input_slice.as_ptr() as *mut f32;
            }
        }

        for (i, output_slice) in buffer.outputs.iter_mut().enumerate() {
            if i < self.output_buffer_ptrs.len() {
                self.output_buffer_ptrs[i] = output_slice.as_mut_ptr();
            }
        }

        // Create audio bus buffers
        let mut input_bus = AudioBusBuffers {
            num_channels: buffer.inputs.len() as i32,
            silence_flags: 0,
            buffers: self.input_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        let mut output_bus = AudioBusBuffers {
            num_channels: buffer.outputs.len() as i32,
            silence_flags: 0,
            buffers: self.output_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        // Reuse pre-allocated event lists (RT-safe: no heap allocation)
        let mut input_event_list = self.input_event_list.take().expect(
            "BUG: input_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        if !midi_events.is_empty() {
            input_event_list.update_from_midi(midi_events);
        } else {
            input_event_list.clear();
        }

        let input_events = if !midi_events.is_empty() {
            input_event_list.as_mut() as *mut EventList as *mut c_void
        } else {
            std::ptr::null_mut()
        };

        // Reuse pre-allocated output event list (RT-safe: no heap allocation)
        let mut output_event_list = self.output_event_list.take().expect(
            "BUG: output_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        output_event_list.clear();

        // Create process data
        let mut process_data = ProcessData {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_32,
            num_samples: num_samples as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            input_param_changes: std::ptr::null_mut(),
            output_param_changes: std::ptr::null_mut(),
            input_events,
            output_events: output_event_list.as_mut() as *mut EventList as *mut c_void,
            context: std::ptr::null_mut(),
        };

        // Process through VST3
        let result =
            unsafe { ((*self.processor_vtable).process)(self.processor, &mut process_data) };

        // On error, clear output buffers to prevent noise/glitches
        if result != K_RESULT_OK {
            for output_slice in buffer.outputs.iter_mut() {
                output_slice.fill(0.0);
            }
            // Return event lists to pool before early exit
            self.input_event_list = Some(input_event_list);
            self.output_event_list = Some(output_event_list);
            return crate::protocol::MidiEventVec::new(); // Return empty MIDI events on error
        }

        // Collect output MIDI events from the plugin (e.g., synths, arpeggiators)
        let midi_out = output_event_list.to_midi_events();

        // Return event lists to pool
        self.input_event_list = Some(input_event_list);
        self.output_event_list = Some(output_event_list);

        midi_out
    }

    /// Process audio with full automation (MIDI + parameter changes)
    pub fn process_with_automation(
        &mut self,
        buffer: &mut AudioBuffer,
        midi_events: &[MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> (
        crate::protocol::MidiEventVec,
        crate::protocol::ParameterChanges,
        crate::protocol::NoteExpressionChanges,
    ) {
        if !self.is_active {
            return (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            );
        }

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            );
        }

        // Update buffer pointers
        for (i, input_slice) in buffer.inputs.iter().enumerate() {
            if i < self.input_buffer_ptrs.len() {
                self.input_buffer_ptrs[i] = input_slice.as_ptr() as *mut f32;
            }
        }

        for (i, output_slice) in buffer.outputs.iter_mut().enumerate() {
            if i < self.output_buffer_ptrs.len() {
                self.output_buffer_ptrs[i] = output_slice.as_mut_ptr();
            }
        }

        // Create audio bus buffers
        let mut input_bus = AudioBusBuffers {
            num_channels: buffer.inputs.len() as i32,
            silence_flags: 0,
            buffers: self.input_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        let mut output_bus = AudioBusBuffers {
            num_channels: buffer.outputs.len() as i32,
            silence_flags: 0,
            buffers: self.output_buffer_ptrs.as_mut_ptr() as *mut *mut c_void,
        };

        // Reuse pre-allocated event lists (RT-safe: no heap allocation)
        let mut input_event_list = self.input_event_list.take().expect(
            "BUG: input_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        if !midi_events.is_empty() || !note_expression.is_empty() {
            input_event_list.update_from_midi_and_expression(midi_events, note_expression);
        } else {
            input_event_list.clear();
        }

        let input_events = if !midi_events.is_empty() || !note_expression.is_empty() {
            input_event_list.as_mut() as *mut EventList as *mut c_void
        } else {
            std::ptr::null_mut()
        };

        // Reuse pre-allocated output event list (RT-safe: no heap allocation)
        let mut output_event_list = self.output_event_list.take().expect(
            "BUG: output_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        output_event_list.clear();

        // Create parameter changes from input
        let mut input_param_changes_box: Option<Box<ParameterChanges>> =
            if !param_changes.is_empty() {
                Some(ParameterChanges::from_protocol(param_changes))
            } else {
                None
            };

        let input_param_changes = if let Some(ref mut changes) = input_param_changes_box {
            changes.as_mut() as *mut ParameterChanges as *mut c_void
        } else {
            std::ptr::null_mut()
        };

        // Create output parameter changes
        let mut output_param_changes = ParameterChanges::new_empty();

        // Create process context from transport info
        let mut process_context =
            ProcessContext::from_transport_info(transport, buffer.sample_rate as f64);

        // Create process data
        let mut process_data = ProcessData {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_32,
            num_samples: num_samples as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            input_param_changes,
            output_param_changes: output_param_changes.as_mut() as *mut ParameterChanges
                as *mut c_void,
            input_events,
            output_events: output_event_list.as_mut() as *mut EventList as *mut c_void,
            context: &mut process_context,
        };

        // Process through VST3
        let result =
            unsafe { ((*self.processor_vtable).process)(self.processor, &mut process_data) };

        // On error, clear output buffers to prevent noise/glitches
        if result != K_RESULT_OK {
            for output_slice in buffer.outputs.iter_mut() {
                output_slice.fill(0.0);
            }
            return (crate::protocol::MidiEventVec::new(), Default::default(), Default::default());
        }

        // Collect outputs
        let midi_output = output_event_list.to_midi_events();
        let param_output = output_param_changes.to_protocol();
        let note_expression_output = output_event_list.to_note_expression_changes();

        // Return event lists to pool
        self.input_event_list = Some(input_event_list);
        self.output_event_list = Some(output_event_list);

        (midi_output, param_output, note_expression_output)
    }

    /// Process audio with f64 buffers through the plugin
    pub fn process_f64(&mut self, buffer: &mut crate::protocol::AudioBuffer64) {
        if !self.is_active {
            return;
        }

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return;
        }

        // Update f64 buffer pointers
        for (i, input_slice) in buffer.inputs.iter().enumerate() {
            if i < self.input_buffer_ptrs_f64.len() {
                self.input_buffer_ptrs_f64[i] = input_slice.as_ptr() as *mut f64;
            }
        }
        for (i, output_slice) in buffer.outputs.iter_mut().enumerate() {
            if i < self.output_buffer_ptrs_f64.len() {
                self.output_buffer_ptrs_f64[i] = output_slice.as_mut_ptr();
            }
        }

        // AudioBusBuffers.buffers is *mut *mut c_void, which works for both f32 and f64
        let mut input_bus = AudioBusBuffers {
            num_channels: buffer.inputs.len() as i32,
            silence_flags: 0,
            buffers: self.input_buffer_ptrs_f64.as_mut_ptr() as *mut *mut c_void,
        };

        let mut output_bus = AudioBusBuffers {
            num_channels: buffer.outputs.len() as i32,
            silence_flags: 0,
            buffers: self.output_buffer_ptrs_f64.as_mut_ptr() as *mut *mut c_void,
        };

        let mut process_data = ProcessData {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_64,
            num_samples: num_samples as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            input_param_changes: std::ptr::null_mut(),
            output_param_changes: std::ptr::null_mut(),
            input_events: std::ptr::null_mut(),
            output_events: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
        };

        let result =
            unsafe { ((*self.processor_vtable).process)(self.processor, &mut process_data) };

        if result != K_RESULT_OK {}
    }

    /// Process audio with full automation using f64 buffers
    pub fn process_with_automation_f64(
        &mut self,
        buffer: &mut crate::protocol::AudioBuffer64,
        midi_events: &[MidiEvent],
        param_changes: &crate::protocol::ParameterChanges,
        note_expression: &crate::protocol::NoteExpressionChanges,
        transport: &crate::protocol::TransportInfo,
    ) -> (
        crate::protocol::MidiEventVec,
        crate::protocol::ParameterChanges,
        crate::protocol::NoteExpressionChanges,
    ) {
        if !self.is_active {
            return (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            );
        }

        let num_samples = buffer.num_samples;
        if num_samples == 0 {
            return (
                smallvec::SmallVec::new(),
                crate::protocol::ParameterChanges::new(),
                crate::protocol::NoteExpressionChanges::new(),
            );
        }

        // Update f64 buffer pointers
        for (i, input_slice) in buffer.inputs.iter().enumerate() {
            if i < self.input_buffer_ptrs_f64.len() {
                self.input_buffer_ptrs_f64[i] = input_slice.as_ptr() as *mut f64;
            }
        }
        for (i, output_slice) in buffer.outputs.iter_mut().enumerate() {
            if i < self.output_buffer_ptrs_f64.len() {
                self.output_buffer_ptrs_f64[i] = output_slice.as_mut_ptr();
            }
        }

        let mut input_bus = AudioBusBuffers {
            num_channels: buffer.inputs.len() as i32,
            silence_flags: 0,
            buffers: self.input_buffer_ptrs_f64.as_mut_ptr() as *mut *mut c_void,
        };

        let mut output_bus = AudioBusBuffers {
            num_channels: buffer.outputs.len() as i32,
            silence_flags: 0,
            buffers: self.output_buffer_ptrs_f64.as_mut_ptr() as *mut *mut c_void,
        };

        // Reuse pre-allocated event lists (RT-safe: no heap allocation)
        let mut input_event_list = self.input_event_list.take().expect(
            "BUG: input_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        if !midi_events.is_empty() || !note_expression.is_empty() {
            input_event_list.update_from_midi_and_expression(midi_events, note_expression);
        } else {
            input_event_list.clear();
        }

        let input_events = if !midi_events.is_empty() || !note_expression.is_empty() {
            input_event_list.as_mut() as *mut EventList as *mut c_void
        } else {
            std::ptr::null_mut()
        };

        // Reuse pre-allocated output event list (RT-safe: no heap allocation)
        let mut output_event_list = self.output_event_list.take().expect(
            "BUG: output_event_list should always be Some (initialized in Vst3Instance::new())",
        );
        output_event_list.clear();

        let mut input_param_changes_box: Option<Box<ParameterChanges>> =
            if !param_changes.is_empty() {
                Some(ParameterChanges::from_protocol(param_changes))
            } else {
                None
            };

        let input_param_changes = if let Some(ref mut changes) = input_param_changes_box {
            changes.as_mut() as *mut ParameterChanges as *mut c_void
        } else {
            std::ptr::null_mut()
        };

        let mut output_param_changes = ParameterChanges::new_empty();
        let mut process_context =
            ProcessContext::from_transport_info(transport, buffer.sample_rate as f64);

        let mut process_data = ProcessData {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_64,
            num_samples: num_samples as i32,
            num_inputs: 1,
            num_outputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            input_param_changes,
            output_param_changes: output_param_changes.as_mut() as *mut ParameterChanges
                as *mut c_void,
            input_events,
            output_events: output_event_list.as_mut() as *mut EventList as *mut c_void,
            context: &mut process_context,
        };

        let result =
            unsafe { ((*self.processor_vtable).process)(self.processor, &mut process_data) };

        // On error, clear output buffers to prevent noise/glitches
        if result != K_RESULT_OK {
            for output_slice in buffer.outputs.iter_mut() {
                output_slice.fill(0.0);
            }
            return (crate::protocol::MidiEventVec::new(), Default::default(), Default::default());
        }

        let midi_output = output_event_list.to_midi_events();
        let param_output = output_param_changes.to_protocol();
        let note_expression_output = output_event_list.to_note_expression_changes();

        // Return event lists to pool
        self.input_event_list = Some(input_event_list);
        self.output_event_list = Some(output_event_list);

        (midi_output, param_output, note_expression_output)
    }

    /// Set sample rate
    pub fn set_sample_rate(&mut self, rate: f32) {
        self.sample_rate = rate;

        let setup = ProcessSetup {
            process_mode: K_REALTIME,
            symbolic_sample_size: K_SAMPLE_32,
            max_samples_per_block: self.block_size as i32,
            sample_rate: rate as f64,
        };

        let result = unsafe { ((*self.processor_vtable).setup_processing)(self.processor, &setup) };

        if result != K_RESULT_OK && result != K_RESULT_TRUE {}
    }

    /// Get parameter count
    pub fn get_parameter_count(&self) -> i32 {
        if let Some(ctrl) = self.controller {
            if let Some(vtable) = self.controller_vtable {
                return unsafe { ((*vtable).get_parameter_count)(ctrl) };
            }
        }
        0
    }

    /// Get parameter value (normalized 0-1)
    pub fn get_parameter(&self, index: u32) -> f64 {
        if let Some(ctrl) = self.controller {
            if let Some(vtable) = self.controller_vtable {
                return unsafe { ((*vtable).get_param_normalized)(ctrl, index) };
            }
        }
        0.0
    }

    /// Set parameter value (normalized 0-1)
    pub fn set_parameter(&mut self, index: u32, value: f64) {
        if let Some(ctrl) = self.controller {
            if let Some(vtable) = self.controller_vtable {
                let _ = unsafe { ((*vtable).set_param_normalized)(ctrl, index, value) };
            }
        }
    }

    /// Save plugin state to byte array
    pub fn get_state(&self) -> Result<Vec<u8>> {
        // VST3 uses IComponent::getState for state serialization
        // For now, we save all parameter values as a simple binary format
        let param_count = self.get_parameter_count();
        let mut state = Vec::with_capacity((param_count as usize + 1) * 8);

        // Write parameter count
        state.extend_from_slice(&param_count.to_le_bytes());

        // Write all parameter values (normalized 0-1, f64 for VST3)
        for i in 0..param_count as u32 {
            let value = self.get_parameter(i);
            state.extend_from_slice(&value.to_le_bytes());
        }

        Ok(state)
    }

    /// Load plugin state from byte array
    pub fn set_state(&mut self, data: &[u8]) -> Result<()> {
        if data.len() < 4 {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Invalid state data".to_string(),
            });
        }

        // Read parameter count
        let param_count = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let expected_size = 4 + (param_count as usize * 8);

        if data.len() != expected_size {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: format!(
                    "State size mismatch: expected {}, got {}",
                    expected_size,
                    data.len()
                ),
            });
        }

        // Read and set all parameter values
        for i in 0..param_count {
            let offset = 4 + (i as usize * 8);
            let value = f64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            self.set_parameter(i as u32, value);
        }

        Ok(())
    }

    /// Check if plugin has editor
    pub fn has_editor(&self) -> bool {
        self.controller.is_some()
    }

    /// Open plugin editor
    ///
    /// # Safety
    ///
    /// The `parent` pointer must be a valid window handle for the target platform.
    pub unsafe fn open_editor(&mut self, parent: *mut c_void) -> Result<(u32, u32)> {
        let ctrl = self.controller.ok_or_else(|| BridgeError::LoadFailed {
            path: PathBuf::from("unknown"),
            stage: LoadStage::Initialization,
            reason: "Plugin has no editor controller".to_string(),
        })?;

        let ctrl_vtable = self.controller_vtable.ok_or_else(|| BridgeError::LoadFailed {
            path: PathBuf::from("unknown"),
            stage: LoadStage::Initialization,
            reason: "Controller vtable missing".to_string(),
        })?;

        // Create view using IEditController::createView
        let view_ptr = unsafe { ((*ctrl_vtable).create_view)(ctrl, std::ptr::null()) };

        if view_ptr.is_null() {
            return Err(BridgeError::LoadFailed {
                path: PathBuf::from("unknown"),
                stage: LoadStage::Initialization,
                reason: "Failed to create plugin view".to_string(),
            });
        }

        let view_vtable = unsafe { *(view_ptr as *const *const IPlugViewVtable) };

        // Attach view to parent window
        #[cfg(target_os = "macos")]
        let platform_type = c"NSView".as_ptr();
        #[cfg(target_os = "windows")]
        let platform_type = c"HWND".as_ptr();
        #[cfg(target_os = "linux")]
        let platform_type = c"X11EmbedWindowID".as_ptr();

        let result = unsafe { ((*view_vtable).attached)(view_ptr, parent, platform_type) };

        if result != K_RESULT_OK {
            return Err(BridgeError::PluginError {
                stage: LoadStage::Initialization,
                code: result,
            });
        }

        // Get view size
        let mut rect = ViewRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };

        let result = unsafe { ((*view_vtable).get_size)(view_ptr, &mut rect) };

        let (width, height) = if result == K_RESULT_OK {
            (
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            )
        } else {
            self.editor_size // Use default
        };

        self.view = Some(view_ptr);
        self.view_vtable = Some(view_vtable);
        self.editor_size = (width, height);

        Ok((width, height))
    }

    /// Close plugin editor
    pub fn close_editor(&mut self) {
        if let (Some(view), Some(vtable)) = (self.view, self.view_vtable) {
            // Detach view
            unsafe {
                ((*vtable).removed)(view);
            }

            // Release view
            let view_unknown = unsafe { *(view as *const *const IUnknownVtable) };
            unsafe {
                ((*view_unknown).release)(view);
            }

            self.view = None;
            self.view_vtable = None;
        }
    }

    /// Editor idle (call periodically to update GUI)
    pub fn editor_idle(&mut self) {
        // VST3 editors don't have an explicit idle callback
        // They use their own event loops
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        // Deactivate plugin
        if self.is_active {
            unsafe {
                let _ = ((*self.processor_vtable).set_processing)(self.processor, 0);
                let _ = ((*self.component_vtable).set_active)(self.component, 0);
            }
        }

        // Terminate component
        unsafe {
            let _ = ((*self.component_vtable).terminate)(self.component);
        }

        // Terminate controller
        if let (Some(ctrl), Some(vtable)) = (self.controller, self.controller_vtable) {
            unsafe {
                let _ = ((*vtable).terminate)(ctrl);
            }
        }

        // Release COM interfaces
        unsafe {
            let vtable = *(self.component as *const *const IUnknownVtable);
            ((*vtable).release)(self.component);

            let vtable = *(self.processor as *const *const IUnknownVtable);
            ((*vtable).release)(self.processor);

            if let Some(ctrl) = self.controller {
                let vtable = *(ctrl as *const *const IUnknownVtable);
                ((*vtable).release)(ctrl);
            }
        }
    }
}
