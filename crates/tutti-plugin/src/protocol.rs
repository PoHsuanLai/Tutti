//! IPC protocol for the plugin bridge process.
//!
//! Public types: `SampleFormat`, `BridgeConfig`, `TransportInfo`, parameter/automation types.
//! Internal types: `HostMessage`, `BridgeMessage`, `IpcMidiEvent`, `AudioBuffer`.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::path::PathBuf;

const MIDI_STACK_CAPACITY: usize = 256;

/// Stack capacity for parameter queues (covers typical automation scenarios)
const PARAM_QUEUE_STACK_CAPACITY: usize = 8;
/// Stack capacity for automation points per parameter
const PARAM_POINT_STACK_CAPACITY: usize = 4;
/// Stack capacity for note expression changes
const NOTE_EXPR_STACK_CAPACITY: usize = 8;

fn default_block_size() -> usize {
    512
}

pub use crate::metadata::PluginMetadata;
pub use tutti_midi_io::MidiEvent;

/// Raw bytes MIDI event for IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcMidiEvent {
    pub frame_offset: usize,
    pub data: [u8; 3],
    pub len: u8,
}

impl IpcMidiEvent {
    pub fn from_bytes(frame_offset: usize, bytes: &[u8]) -> Self {
        let mut data = [0u8; 3];
        let len = bytes.len().min(3);
        data[..len].copy_from_slice(&bytes[..len]);
        Self {
            frame_offset,
            data,
            len: len as u8,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    pub fn to_midi_event(&self) -> Option<MidiEvent> {
        MidiEvent::from_bytes(self.as_bytes()).ok().map(|mut e| {
            e.frame_offset = self.frame_offset;
            e
        })
    }
}

impl From<&MidiEvent> for IpcMidiEvent {
    fn from(event: &MidiEvent) -> Self {
        let bytes = event.to_bytes();
        Self::from_bytes(event.frame_offset, &bytes)
    }
}

impl From<MidiEvent> for IpcMidiEvent {
    fn from(event: MidiEvent) -> Self {
        Self::from(&event)
    }
}

pub type IpcMidiEventVec = SmallVec<[IpcMidiEvent; MIDI_STACK_CAPACITY]>;
pub type MidiEventVec = SmallVec<[MidiEvent; MIDI_STACK_CAPACITY]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleFormat {
    Float32,
    Float64,
}

#[allow(clippy::derivable_impls)]
impl Default for SampleFormat {
    fn default() -> Self {
        SampleFormat::Float32
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ParameterPoint {
    pub sample_offset: i32,
    pub value: f64,
}

pub type ParameterPointVec = SmallVec<[ParameterPoint; PARAM_POINT_STACK_CAPACITY]>;

/// `param_id` is the format-native identifier (VST3 ParamID, CLAP clap_id, or VST2 index).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterQueue {
    pub param_id: u32,
    pub points: ParameterPointVec,
}

impl ParameterQueue {
    pub fn new(param_id: u32) -> Self {
        Self {
            param_id,
            points: SmallVec::new(),
        }
    }

    pub fn add_point(&mut self, sample_offset: i32, value: f64) {
        self.points.push(ParameterPoint {
            sample_offset,
            value,
        });
    }
}

pub type ParameterQueueVec = SmallVec<[ParameterQueue; PARAM_QUEUE_STACK_CAPACITY]>;

/// Collection of parameter automation changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterChanges {
    pub queues: ParameterQueueVec,
}

impl ParameterChanges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_queue(&mut self, queue: ParameterQueue) {
        self.queues.push(queue);
    }

    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterFlags {
    pub automatable: bool,
    pub read_only: bool,
    pub wrap: bool,
    pub is_bypass: bool,
    pub hidden: bool,
}

/// `id` is the format-native identifier (VST3 ParamID, CLAP clap_id, or VST2 index).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInfo {
    pub id: u32,
    pub name: String,
    pub unit: String,
    pub min_value: f64,
    pub max_value: f64,
    pub default_value: f64,
    pub step_count: u32,
    pub flags: ParameterFlags,
}

impl ParameterInfo {
    pub fn new(id: u32, name: String) -> Self {
        Self {
            id,
            name,
            unit: String::new(),
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            step_count: 0,
            flags: ParameterFlags::default(),
        }
    }

    /// Convert to ParameterRange for automation integration.
    ///
    /// Infers the scaling type from step_count and unit string:
    /// - `step_count == 1`: Toggle (on/off)
    /// - `step_count > 1`: Integer steps
    /// - Unit contains "dB" or "Hz": Logarithmic
    /// - Otherwise: Linear
    pub fn to_range(&self) -> tutti_core::ParameterRange {
        use tutti_core::{ParameterRange, ParameterScale};

        let scale = if self.step_count == 1 {
            // Binary toggle
            ParameterScale::Toggle
        } else if self.step_count > 1 {
            // Discrete integer steps
            ParameterScale::Integer
        } else if self.unit.contains("dB") || self.unit.contains("Hz") || self.unit.contains("hz") {
            // Frequency or decibel parameters work better with log scaling
            ParameterScale::Logarithmic
        } else {
            ParameterScale::Linear
        };

        // Handle logarithmic with non-positive min
        let scale = if matches!(scale, ParameterScale::Logarithmic) && self.min_value <= 0.0 {
            // Fallback to linear if min is non-positive (log requires positive values)
            ParameterScale::Linear
        } else {
            scale
        };

        ParameterRange::new(
            self.min_value as f32,
            self.max_value as f32,
            self.default_value as f32,
            scale,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteExpressionType {
    Volume,
    Pan,
    Tuning,
    Vibrato,
    Brightness,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NoteExpressionValue {
    pub sample_offset: i32,
    pub note_id: i32,
    pub expression_type: NoteExpressionType,
    pub value: f64,
}

pub type NoteExpressionVec = SmallVec<[NoteExpressionValue; NOTE_EXPR_STACK_CAPACITY]>;

/// Collection of note expression changes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteExpressionChanges {
    pub changes: NoteExpressionVec,
}

impl NoteExpressionChanges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_change(&mut self, change: NoteExpressionValue) {
        self.changes.push(change);
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TransportInfo {
    pub playing: bool,
    pub recording: bool,
    pub cycle_active: bool,
    pub tempo: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub position_samples: i64,
    pub position_quarters: f64,
    pub bar_position_quarters: f64,
    pub cycle_start_quarters: f64,
    pub cycle_end_quarters: f64,
}

impl Default for TransportInfo {
    fn default() -> Self {
        Self {
            playing: false,
            recording: false,
            cycle_active: false,
            tempo: 120.0,
            time_sig_numerator: 4,
            time_sig_denominator: 4,
            position_samples: 0,
            position_quarters: 0.0,
            bar_position_quarters: 0.0,
            cycle_start_quarters: 0.0,
            cycle_end_quarters: 0.0,
        }
    }
}

pub struct AudioBuffer<'a, T = f32> {
    pub inputs: &'a [&'a [T]],
    pub outputs: &'a mut [&'a mut [T]],
    pub num_samples: usize,
    pub sample_rate: f64,
}

pub type AudioBuffer32<'a> = AudioBuffer<'a, f32>;
pub type AudioBuffer64<'a> = AudioBuffer<'a, f64>;

/// Data for `HostMessage::ProcessAudioMidi`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessAudioMidiData {
    pub buffer_id: u32,
    pub num_samples: usize,
    pub midi_events: IpcMidiEventVec,
}

/// Data for `HostMessage::ProcessAudioFull`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessAudioFullData {
    pub buffer_id: u32,
    pub num_samples: usize,
    pub midi_events: IpcMidiEventVec,
    pub param_changes: ParameterChanges,
    pub note_expression: NoteExpressionChanges,
    pub transport: TransportInfo,
}

/// Host to bridge message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostMessage {
    LoadPlugin {
        path: PathBuf,
        sample_rate: f64,
        #[serde(default = "default_block_size")]
        block_size: usize,
        #[serde(default)]
        preferred_format: SampleFormat,
        /// Shared memory name created by the client for audio I/O.
        #[serde(default)]
        shm_name: String,
    },
    UnloadPlugin,
    ProcessAudio {
        buffer_id: u32,
        num_samples: usize,
    },
    ProcessAudioMidi(Box<ProcessAudioMidiData>),
    ProcessAudioFull(Box<ProcessAudioFullData>),
    SetParameter {
        param_id: u32,
        value: f32,
    },
    GetParameter {
        param_id: u32,
    },
    GetParameterList,
    GetParameterInfo {
        param_id: u32,
    },
    SetSampleRate {
        rate: f64,
    },
    Reset,
    SaveState,
    LoadState {
        data: Vec<u8>,
    },
    OpenEditor {
        parent_handle: u64,
    },
    CloseEditor,
    EditorIdle,
    Shutdown,
}

/// Data for `BridgeMessage::AudioProcessedMidi`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessedMidiData {
    pub latency_us: u64,
    pub midi_output: IpcMidiEventVec,
}

/// Data for `BridgeMessage::AudioProcessedFull`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioProcessedFullData {
    pub latency_us: u64,
    pub midi_output: IpcMidiEventVec,
    pub param_output: ParameterChanges,
    pub note_expression_output: NoteExpressionChanges,
}

/// Bridge to host message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeMessage {
    PluginLoaded { metadata: Box<PluginMetadata> },
    PluginUnloaded,
    AudioProcessed { latency_us: u64 },
    AudioProcessedMidi(Box<AudioProcessedMidiData>),
    AudioProcessedFull(Box<AudioProcessedFullData>),
    ParameterValue { value: Option<f32> },
    ParameterList { parameters: Vec<ParameterInfo> },
    ParameterInfoResponse { info: Option<ParameterInfo> },
    StateData { data: Vec<u8> },
    EditorOpened { width: u32, height: u32 },
    EditorClosed,
    ParameterChanged { index: i32, value: f32 },
    Error { message: String },
    Ready,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedBuffer {
    pub id: u32,
    pub size: usize,
    pub channels: usize,
    pub samples: usize,
    pub shm_name: String,
    #[serde(default)]
    pub sample_format: SampleFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub socket_path: PathBuf,
    pub shm_prefix: String,
    pub max_buffer_size: usize,
    pub timeout_ms: u64,
    #[serde(default)]
    pub preferred_format: SampleFormat,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            socket_path: std::env::temp_dir().join("dawai-bridge.sock"),
            shm_prefix: "dawai_audio_".to_string(),
            max_buffer_size: 8192,
            timeout_ms: 5000,
            preferred_format: SampleFormat::Float32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutti_core::ParameterScale;

    #[test]
    fn test_message_serialization() {
        let msg = HostMessage::LoadPlugin {
            path: PathBuf::from("/test/plugin.vst3"),
            sample_rate: 44100.0,
            block_size: 512,
            preferred_format: SampleFormat::Float32,
            shm_name: String::new(),
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::LoadPlugin {
                path, sample_rate, ..
            } => {
                assert_eq!(path, PathBuf::from("/test/plugin.vst3"));
                assert_eq!(sample_rate, 44100.0);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_ipc_midi_event_roundtrip() {
        let event = MidiEvent::note_on_builder(60, 100)
            .channel(0)
            .offset(128)
            .build();
        let ipc_event = IpcMidiEvent::from(&event);
        assert_eq!(ipc_event.frame_offset, 128);
        assert_eq!(ipc_event.len, 3);

        let restored = ipc_event.to_midi_event().unwrap();
        assert_eq!(restored.frame_offset, 128);
        assert!(restored.is_note_on());
        assert_eq!(restored.note(), Some(60));
        assert_eq!(restored.velocity(), Some(100));
    }

    #[test]
    fn test_midi_message_serialization() {
        let midi_events: IpcMidiEventVec = vec![
            MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
            MidiEvent::note_on_builder(64, 100)
                .channel(0)
                .offset(128)
                .build(),
            MidiEvent::cc_builder(7, 64).channel(0).offset(256).build(),
        ]
        .iter()
        .map(IpcMidiEvent::from)
        .collect();

        let msg = HostMessage::ProcessAudioMidi(Box::new(ProcessAudioMidiData {
            buffer_id: 42,
            num_samples: 512,
            midi_events,
        }));

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::ProcessAudioMidi(data) => {
                let buffer_id = data.buffer_id;
                let num_samples = data.num_samples;
                let decoded_events = data.midi_events;
                assert_eq!(buffer_id, 42);
                assert_eq!(num_samples, 512);
                assert_eq!(decoded_events.len(), 3);

                let events: Vec<MidiEvent> = decoded_events
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                assert_eq!(events.len(), 3);
                assert_eq!(events[0].frame_offset, 0);
                assert!(events[0].is_note_on());
                assert_eq!(events[0].note(), Some(60));
                assert_eq!(events[1].frame_offset, 128);
                assert!(events[1].is_note_on());
                assert_eq!(events[1].note(), Some(64));
                assert_eq!(events[2].frame_offset, 256);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_midi_output_response_serialization() {
        let midi_output: IpcMidiEventVec = vec![
            MidiEvent::note_off_builder(60)
                .channel(0)
                .offset(512)
                .build(),
            MidiEvent::note_off_builder(64)
                .channel(0)
                .offset(640)
                .build(),
        ]
        .iter()
        .map(IpcMidiEvent::from)
        .collect();

        let msg = BridgeMessage::AudioProcessedMidi(Box::new(AudioProcessedMidiData {
            latency_us: 1500,
            midi_output,
        }));

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: BridgeMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            BridgeMessage::AudioProcessedMidi(data) => {
                assert_eq!(data.latency_us, 1500);
                let decoded_output = data.midi_output;
                assert_eq!(decoded_output.len(), 2);

                let events: Vec<MidiEvent> = decoded_output
                    .iter()
                    .filter_map(|e| e.to_midi_event())
                    .collect();
                assert!(events[0].is_note_off());
                assert!(events[1].is_note_off());
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_load_plugin_f64_serialization() {
        let msg = HostMessage::LoadPlugin {
            path: PathBuf::from("/test/reverb.vst3"),
            sample_rate: 96000.0,
            block_size: 1024,
            preferred_format: SampleFormat::Float64,
            shm_name: "test_shm".to_string(),
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                block_size,
                preferred_format,
                ..
            } => {
                assert_eq!(path, PathBuf::from("/test/reverb.vst3"));
                assert_eq!(sample_rate, 96000.0);
                assert_eq!(block_size, 1024);
                assert_eq!(preferred_format, SampleFormat::Float64);
            }
            _ => panic!("Wrong message type"),
        }
    }

    // --- ParameterInfo::to_range() ---

    #[test]
    fn test_to_range_toggle() {
        let mut info = ParameterInfo::new(1, "Bypass".to_string());
        info.step_count = 1;
        let range = info.to_range();
        assert_eq!(range.scale, ParameterScale::Toggle);
    }

    #[test]
    fn test_to_range_integer() {
        let mut info = ParameterInfo::new(2, "Algorithm".to_string());
        info.step_count = 5;
        let range = info.to_range();
        assert_eq!(range.scale, ParameterScale::Integer);
    }

    #[test]
    fn test_to_range_logarithmic_db() {
        let mut info = ParameterInfo::new(3, "Gain".to_string());
        info.unit = "dB".to_string();
        info.min_value = 0.001;
        info.max_value = 10.0;
        let range = info.to_range();
        assert_eq!(range.scale, ParameterScale::Logarithmic);
    }

    #[test]
    fn test_to_range_logarithmic_hz() {
        let mut info = ParameterInfo::new(4, "Cutoff".to_string());
        info.unit = "Hz".to_string();
        info.min_value = 20.0;
        info.max_value = 20000.0;
        let range = info.to_range();
        assert_eq!(range.scale, ParameterScale::Logarithmic);
    }

    #[test]
    fn test_to_range_log_fallback_non_positive_min() {
        let mut info = ParameterInfo::new(5, "Freq".to_string());
        info.unit = "Hz".to_string();
        info.min_value = 0.0;
        info.max_value = 20000.0;
        let range = info.to_range();
        // Falls back to Linear because log requires positive min
        assert_eq!(range.scale, ParameterScale::Linear);
    }

    #[test]
    fn test_to_range_linear_default() {
        let info = ParameterInfo::new(6, "Mix".to_string());
        let range = info.to_range();
        assert_eq!(range.scale, ParameterScale::Linear);
    }

    #[test]
    fn test_to_range_values_preserved() {
        let mut info = ParameterInfo::new(7, "Volume".to_string());
        info.min_value = -96.0;
        info.max_value = 6.0;
        info.default_value = -12.0;
        let range = info.to_range();
        assert_eq!(range.min, -96.0);
        assert_eq!(range.max, 6.0);
        assert_eq!(range.default, -12.0);
    }

    // --- Collections ---

    #[test]
    fn test_parameter_changes_add_queue() {
        let mut changes = ParameterChanges::new();
        assert!(changes.is_empty());

        let mut queue = ParameterQueue::new(42);
        queue.add_point(0, 0.5);
        queue.add_point(128, 0.8);
        changes.add_queue(queue);

        assert!(!changes.is_empty());
        assert_eq!(changes.queues.len(), 1);
        assert_eq!(changes.queues[0].param_id, 42);
        assert_eq!(changes.queues[0].points.len(), 2);
    }

    #[test]
    fn test_note_expression_add_change() {
        let mut expr = NoteExpressionChanges::new();
        assert!(expr.is_empty());

        expr.add_change(NoteExpressionValue {
            sample_offset: 0,
            note_id: 1,
            expression_type: NoteExpressionType::Tuning,
            value: 0.5,
        });

        assert!(!expr.is_empty());
        assert_eq!(expr.changes.len(), 1);
        assert_eq!(expr.changes[0].expression_type, NoteExpressionType::Tuning);
    }

    #[test]
    fn test_transport_info_default() {
        let info = TransportInfo::default();
        assert_eq!(info.tempo, 120.0);
        assert_eq!(info.time_sig_numerator, 4);
        assert_eq!(info.time_sig_denominator, 4);
        assert!(!info.playing);
        assert!(!info.recording);
    }
}
