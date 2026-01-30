//! Bridge protocol - messages between host and plugin process
//!
//! This defines the IPC protocol for communication between the main DAWAI
//! process and the sandboxed plugin bridge process.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Use local metadata type for IPC
pub use crate::metadata::PluginMetadata;

// Re-export MIDI event from tutti-midi (now serializable!)
pub use tutti_midi::MidiEvent;

/// Audio sample format
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SampleFormat {
    /// 32-bit floating point (most common)
    Float32,
    /// 64-bit floating point (high precision)
    Float64,
}

#[allow(clippy::derivable_impls)]
impl Default for SampleFormat {
    fn default() -> Self {
        SampleFormat::Float32
    }
}

/// Single parameter automation point
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ParameterPoint {
    /// Sample offset within the current buffer (0 = first sample)
    pub sample_offset: i32,
    /// Normalized parameter value (0.0 to 1.0)
    pub value: f64,
}

/// Parameter automation queue for a single parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterQueue {
    /// Parameter ID (VST3 uses u32, called ParamID)
    pub param_id: u32,
    /// Automation points sorted by sample_offset
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

/// Collection of parameter changes for a processing block
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParameterChanges {
    /// All parameter queues (one per parameter that changed)
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

/// Note expression type (VST3-style per-note modulation)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoteExpressionType {
    /// Volume (0.0 = silent, 0.5 = default, 1.0 = max)
    Volume,
    /// Pan (-1.0 = left, 0.0 = center, 1.0 = right)
    Pan,
    /// Tuning in semitones (-120.0 to +120.0, default 0.0)
    Tuning,
    /// Vibrato depth (0.0 = none, 1.0 = max)
    Vibrato,
    /// Brightness/timbre (0.0 = dark, 0.5 = default, 1.0 = bright)
    Brightness,
}

/// Single note expression value change
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NoteExpressionValue {
    /// Sample offset within the current buffer
    pub sample_offset: i32,
    /// Note ID (for VST3, this is the unique note identifier)
    pub note_id: i32,
    /// Expression type
    pub expression_type: NoteExpressionType,
    /// Normalized value (meaning depends on expression_type)
    pub value: f64,
}

/// Collection of note expression changes for a processing block
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NoteExpressionChanges {
    /// All note expression changes
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

/// Transport and timing information for audio processing
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TransportInfo {
    /// Is transport playing?
    pub playing: bool,
    /// Is recording active?
    pub recording: bool,
    /// Is cycle/loop active?
    pub cycle_active: bool,
    /// Tempo in BPM
    pub tempo: f64,
    /// Time signature numerator (e.g., 4 in 4/4)
    pub time_sig_numerator: i32,
    /// Time signature denominator (e.g., 4 in 4/4)
    pub time_sig_denominator: i32,
    /// Position in samples from project start
    pub position_samples: i64,
    /// Musical position in quarter notes
    pub position_quarters: f64,
    /// Last bar start position in quarter notes
    pub bar_position_quarters: f64,
    /// Cycle/loop start in quarter notes (if cycle active)
    pub cycle_start_quarters: f64,
    /// Cycle/loop end in quarter notes (if cycle active)
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

/// Audio buffer for plugin processing (generic over sample type)
///
/// This is a simple buffer type used internally by the plugin loaders
/// in the bridge process. It's not the same as FunDSP's buffer types.
pub struct AudioBuffer<'a, T = f32> {
    /// Input channel slices
    pub inputs: &'a [&'a [T]],
    /// Output channel slices (mutable)
    pub outputs: &'a mut [&'a mut [T]],
    /// Number of samples per channel
    pub num_samples: usize,
    /// Sample rate
    pub sample_rate: f32,
}

/// Type alias for 32-bit audio buffer (most common)
pub type AudioBuffer32<'a> = AudioBuffer<'a, f32>;

/// Type alias for 64-bit audio buffer (high precision)
pub type AudioBuffer64<'a> = AudioBuffer<'a, f64>;

/// Message from host to bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostMessage {
    /// Load a plugin from path
    LoadPlugin {
        path: PathBuf,
        sample_rate: f32,
        /// Preferred sample format (f32 or f64). Server will negotiate based on plugin capability.
        #[serde(default)]
        preferred_format: SampleFormat,
    },

    /// Unload the current plugin
    UnloadPlugin,

    /// Process audio (buffer ID in shared memory)
    ProcessAudio { buffer_id: u32, num_samples: usize },

    /// Process audio with MIDI events (sample-accurate)
    ProcessAudioMidi {
        buffer_id: u32,
        num_samples: usize,
        midi_events: Vec<MidiEvent>,
    },

    /// Process audio with full automation (MIDI + parameter changes + transport + note expression)
    ProcessAudioFull {
        buffer_id: u32,
        num_samples: usize,
        midi_events: Vec<MidiEvent>,
        param_changes: ParameterChanges,
        note_expression: NoteExpressionChanges,
        transport: TransportInfo,
    },

    /// Set a parameter
    SetParameter { id: String, value: f32 },

    /// Get a parameter value
    GetParameter { id: String },

    /// Set sample rate
    SetSampleRate { rate: f32 },

    /// Reset plugin state
    Reset,

    /// Save plugin state
    SaveState,

    /// Load plugin state
    LoadState { data: Vec<u8> },

    /// Open editor GUI
    OpenEditor { parent_handle: u64 },

    /// Close editor GUI
    CloseEditor,

    /// Editor idle (for legacy plugins)
    EditorIdle,

    /// Shutdown the bridge
    Shutdown,
}

/// Message from bridge to host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BridgeMessage {
    /// Plugin loaded successfully
    PluginLoaded { metadata: Box<PluginMetadata> },

    /// Plugin unloaded
    PluginUnloaded,

    /// Audio processing complete
    AudioProcessed { latency_us: u64 },

    /// Audio processing complete with MIDI output
    AudioProcessedMidi {
        latency_us: u64,
        midi_output: Vec<MidiEvent>,
    },

    /// Audio processing complete with full output (MIDI + parameter changes + note expression)
    AudioProcessedFull {
        latency_us: u64,
        midi_output: Vec<MidiEvent>,
        param_output: ParameterChanges,
        note_expression_output: NoteExpressionChanges,
    },

    /// Parameter value response
    ParameterValue { value: Option<f32> },

    /// Plugin state data
    StateData { data: Vec<u8> },

    /// Editor opened
    EditorOpened { width: u32, height: u32 },

    /// Editor closed
    EditorClosed,

    /// Parameter changed by plugin (automation)
    ParameterChanged { index: i32, value: f32 },

    /// Error occurred
    Error { message: String },

    /// Bridge ready
    Ready,

    /// Bridge shutting down
    Shutdown,
}

/// Shared memory buffer descriptor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedBuffer {
    /// Unique buffer ID
    pub id: u32,

    /// Size in bytes
    pub size: usize,

    /// Number of channels
    pub channels: usize,

    /// Number of samples per channel
    pub samples: usize,

    /// Shared memory name/path
    pub shm_name: String,

    /// Sample format used in this buffer
    #[serde(default)]
    pub sample_format: SampleFormat,
}

/// Bridge configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeConfig {
    /// Socket path for IPC
    pub socket_path: PathBuf,

    /// Shared memory prefix
    pub shm_prefix: String,

    /// Maximum buffer size
    pub max_buffer_size: usize,

    /// Timeout for operations (milliseconds)
    pub timeout_ms: u64,

    /// Preferred sample format (f32 or f64). Default: Float32.
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
    fn test_midi_message_serialization() {
        // Create MIDI events
        let midi_events = vec![
            MidiEvent::note_on_builder(60, 100)
                .channel(0)
                .offset(0)
                .build(),
            MidiEvent::note_on_builder(64, 100)
                .channel(0)
                .offset(128)
                .build(),
            MidiEvent::cc_builder(7, 64).channel(0).offset(256).build(),
        ];

        let msg = HostMessage::ProcessAudioMidi {
            buffer_id: 42,
            num_samples: 512,
            midi_events: midi_events.clone(),
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

                // Verify first note on
                assert_eq!(decoded_events[0].frame_offset, 0);
                assert!(decoded_events[0].is_note_on());
                assert_eq!(decoded_events[0].note(), Some(60));

                // Verify second note on
                assert_eq!(decoded_events[1].frame_offset, 128);
                assert!(decoded_events[1].is_note_on());
                assert_eq!(decoded_events[1].note(), Some(64));

                // Verify CC
                assert_eq!(decoded_events[2].frame_offset, 256);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_midi_output_response_serialization() {
        let midi_output = vec![
            MidiEvent::note_off_builder(60)
                .channel(0)
                .offset(512)
                .build(),
            MidiEvent::note_off_builder(64)
                .channel(0)
                .offset(640)
                .build(),
        ];

        let msg = BridgeMessage::AudioProcessedMidi {
            latency_us: 1500,
            midi_output: midi_output.clone(),
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
                assert!(decoded_output[0].is_note_off());
                assert!(decoded_output[1].is_note_off());
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_load_plugin_f64_serialization() {
        let msg = HostMessage::LoadPlugin {
            path: PathBuf::from("/test/reverb.vst3"),
            sample_rate: 96000.0,
            preferred_format: SampleFormat::Float64,
        };

        let encoded = bincode::serialize(&msg).unwrap();
        let decoded: HostMessage = bincode::deserialize(&encoded).unwrap();

        match decoded {
            HostMessage::LoadPlugin {
                path,
                sample_rate,
                preferred_format,
            } => {
                assert_eq!(path, PathBuf::from("/test/reverb.vst3"));
                assert_eq!(sample_rate, 96000.0);
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
