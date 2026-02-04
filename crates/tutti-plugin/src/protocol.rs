//! IPC protocol for the plugin bridge process.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::path::PathBuf;

const MIDI_STACK_CAPACITY: usize = 256;

fn default_block_size() -> usize {
    512
}

pub use crate::metadata::PluginMetadata;
pub use tutti_midi_io::MidiEvent;

/// IPC-serializable MIDI event (raw bytes format for cross-process communication).
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

/// Parameter automation queue.
///
/// `param_id` is the format-native identifier (VST3 ParamID, CLAP clap_id, or VST2 index).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterQueue {
    pub param_id: u32,
    pub points: Vec<ParameterPoint>,
}

impl ParameterQueue {
    pub fn new(param_id: u32) -> Self {
        Self {
            param_id,
            points: Vec::new(),
        }
    }

    pub fn add_point(&mut self, sample_offset: i32, value: f64) {
        self.points.push(ParameterPoint {
            sample_offset,
            value,
        });
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterChanges {
    pub queues: Vec<ParameterQueue>,
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

/// Parameter metadata.
///
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteExpressionChanges {
    pub changes: Vec<NoteExpressionValue>,
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
    },
    UnloadPlugin,
    ProcessAudio {
        buffer_id: u32,
        num_samples: usize,
    },
    ProcessAudioMidi {
        buffer_id: u32,
        num_samples: usize,
        midi_events: IpcMidiEventVec,
    },
    ProcessAudioFull {
        buffer_id: u32,
        num_samples: usize,
        midi_events: IpcMidiEventVec,
        param_changes: ParameterChanges,
        note_expression: NoteExpressionChanges,
        transport: TransportInfo,
    },
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

/// Bridge to host message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeMessage {
    PluginLoaded {
        metadata: Box<PluginMetadata>,
    },
    PluginUnloaded,
    AudioProcessed {
        latency_us: u64,
    },
    AudioProcessedMidi {
        latency_us: u64,
        midi_output: IpcMidiEventVec,
    },
    AudioProcessedFull {
        latency_us: u64,
        midi_output: IpcMidiEventVec,
        param_output: ParameterChanges,
        note_expression_output: NoteExpressionChanges,
    },
    ParameterValue {
        value: Option<f32>,
    },
    ParameterList {
        parameters: Vec<ParameterInfo>,
    },
    ParameterInfoResponse {
        info: Option<ParameterInfo>,
    },
    StateData {
        data: Vec<u8>,
    },
    EditorOpened {
        width: u32,
        height: u32,
    },
    EditorClosed,
    ParameterChanged {
        index: i32,
        value: f32,
    },
    Error {
        message: String,
    },
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

    #[test]
    fn test_message_serialization() {
        let msg = HostMessage::LoadPlugin {
            path: PathBuf::from("/test/plugin.vst3"),
            sample_rate: 44100.0,
            block_size: 512,
            preferred_format: SampleFormat::Float32,
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
    fn test_bridge_config_default() {
        let config = BridgeConfig::default();
        assert_eq!(config.max_buffer_size, 8192);
        assert_eq!(config.timeout_ms, 5000);
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

        let msg = HostMessage::ProcessAudioMidi {
            buffer_id: 42,
            num_samples: 512,
            midi_events,
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::ProcessAudioMidi {
                buffer_id,
                num_samples,
                midi_events: decoded_events,
            } => {
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

        let msg = BridgeMessage::AudioProcessedMidi {
            latency_us: 1500,
            midi_output,
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: BridgeMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            BridgeMessage::AudioProcessedMidi {
                latency_us,
                midi_output: decoded_output,
            } => {
                assert_eq!(latency_us, 1500);
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
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                block_size,
                preferred_format,
            } => {
                assert_eq!(path, PathBuf::from("/test/reverb.vst3"));
                assert_eq!(sample_rate, 96000.0);
                assert_eq!(block_size, 1024);
                assert_eq!(preferred_format, SampleFormat::Float64);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_sample_format_default() {
        assert_eq!(SampleFormat::default(), SampleFormat::Float32);
    }

    #[test]
    fn test_bridge_config_default_format() {
        let config = BridgeConfig::default();
        assert_eq!(config.preferred_format, SampleFormat::Float32);
    }
}
