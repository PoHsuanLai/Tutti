//! # Tutti - Real-time Audio Engine
//!
//! Complete audio engine built from modular subsystems.
//!
//! ## Architecture
//!
//! Tutti is an umbrella crate that coordinates:
//! - **tutti-core** - Audio graph runtime (Net, Transport, Metering, PDC)
//! - **tutti-midi** - MIDI subsystem (I/O, MPE, MIDI 2.0, CC mapping)
//! - **tutti-sampler** - Sample playback (Butler, streaming, recording, time-stretch)
//! - **tutti-synth** - Software synthesizers (SoundFont, polyphonic synth, wavetable)
//! - **tutti-dsp** - DSP nodes (LFO, dynamics, envelope follower, spatial audio)
//! - **tutti-plugin** - Plugin hosting (VST2, VST3, CLAP)
//! - **tutti-neural** - Neural audio (GPU synthesis and effects)
//! - **tutti-analysis** - Audio analysis (waveform, transient, pitch, correlation)
//! - **tutti-export** - Offline rendering and export
//!
//! ## Quick Start
//!
//! ```ignore
//! use tutti::prelude::*;
//!
//! // Create engine (sample rate determined by audio device)
//! let engine = TuttiEngine::builder()
//!     .build()?;
//!
//! // Load resources with fluent builders (returns AudioUnit)
//! let piano = engine.sf2("piano.sf2").preset(0).build()?;
//! let sample = engine.wav("kick.wav").build()?;
//!
//! // Add nodes to graph
//! let piano_id = engine.graph_mut(|net| net.add(piano).master());
//! let sample_id = engine.graph_mut(|net| net.add(sample).master());
//!
//! // Or create DSP nodes directly
//! engine.graph_mut(|net| {
//!     let osc = net.add(sine_hz::<f32>(440.0)).id();
//!     let filter = net.add(lowpass_hz::<f32>(2000.0, 1.0)).id();
//!     chain!(net, osc, filter => output);
//! });
//!
//! // Control transport
//! engine.transport().play();
//!
//! // Play MIDI notes
//! engine.note_on(piano_id, Note::C4, 100);
//! ```
//!
//! ## Built-in Audio Nodes
//!
//! Tutti provides many AudioUnit implementations out of the box:
//!
//! - **Synths**: `SoundFontUnit` (sample-based), voice allocator, modulation matrix
//! - **Samplers**: `SamplerUnit`, `StreamingSamplerUnit`, `TimeStretchUnit`
//! - **DSP**: `LfoNode`, sidechain compressor/gate
//! - **Spatial**: `BinauralPannerNode` (HRTF), `SpatialPannerNode` (VBAP)
//! - **Effects**: All FunDSP nodes (`lowpass_hz`, `reverb_stereo`, 100+ more)
//!
//! See [BUILTIN_NODES.md](https://github.com/PoHsuanLai/Tutti/blob/main/BUILTIN_NODES.md)
//! for complete reference with examples.
//!
//! ## Feature Flags
//!
//! - `default` - Core audio engine
//! - `full` - Everything enabled
//! - `midi` - MIDI subsystem
//! - `sampler` - Sample playback and recording
//! - `plugin` - Plugin hosting
//! - `neural` - Neural audio
//! - `analysis` - Audio analysis tools
//! - `export` - Offline rendering

/// Re-export of tutti-core for direct access
pub use tutti_core as core;

// Centralized error type (wraps all subsystem errors)
mod error;
pub use error::{Error, Result};

// Core types
pub use tutti_core::{
    // Click (metronome as AudioUnit)
    click,
    AtomicAmplitude,
    AtomicDouble,
    AtomicFlag,

    // Lock-free primitives
    AtomicFloat,
    AtomicStereoAnalysis,
    // Audio graph
    AudioUnit,
    BufferMut,
    BufferRef,
    ClickNode,
    ClickSettings,
    ClickState,
    CpuMeter,
    CpuMetrics,

    Direction,
    EventId,
    Fade,
    // Metering (includes LUFS!)
    MeteringHandle,
    MeteringManager,
    MetronomeHandle,
    MetronomeMode,
    MotionState,
    NetBackend,

    NodeConstructor,
    NodeId,
    // Node introspection
    NodeInfo,
    NodeParamValue,
    NodeParams,
    NodeRegistryError,
    Params,
    PdcDelayUnit,

    // PDC
    PdcManager,
    PdcState,
    ReplayMode,

    // Sequencer
    Sequencer,
    Shared,
    SignalFrame,
    Source,
    StereoAnalysisSnapshot,
    TempoMap,
    TimeSignature,
    TransportClock,

    // Transport
    TransportHandle,
    TransportManager,
    TransportReader,
    TuttiNet,
    // Audio data
    Wave,
    BBT,
};

// Atomic types (from core:: via tutti-core, no_std compatible)
pub use tutti_core::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

// Sample types
pub use tutti_core::{Sample, F32, F64};

// MIDI I/O subsystem
#[cfg(feature = "midi")]
pub use tutti_midi_io as midi;

#[cfg(feature = "midi")]
pub use tutti_midi_io::{
    CCMapping, CCMappingManager, CCNumber, CCProcessResult, CCTarget, Channel, ChannelVoiceMsg,
    ControlChange, MappingId, MidiChannel, MidiEvent, MidiHandle, MidiMsg, MidiSystem,
    MidiSystemBuilder, Note, PortInfo, RawMidiEvent,
};

#[cfg(feature = "midi-hardware")]
pub use tutti_midi_io::{MidiInputDevice, MidiOutputDevice};

#[cfg(feature = "mpe")]
pub use tutti_midi_io::{MpeHandle, MpeMode, MpeZone, MpeZoneConfig};

#[cfg(feature = "midi2")]
pub use tutti_midi_io::{Midi2Event, Midi2Handle, Midi2MessageType, UnifiedMidiEvent};

#[cfg(feature = "midi")]
pub use tutti_core::{
    MidiEventBuilder, MidiRegistry, MidiRoutingTable, MidiSnapshotReader, MidiSource,
};

// Sampler subsystem (optional)
#[cfg(feature = "sampler")]
pub use tutti_sampler as sampler;

#[cfg(feature = "sampler")]
pub use tutti_sampler::{
    AudioInput, AudioInputBackend, ImportHandle, ImportStatus, PlayDirection, SamplerHandle,
    SamplerSystem, SamplerSystemBuilder, SamplerUnit, StreamingSamplerUnit, TimeStretchUnit,
    Varispeed,
};

// Time stretch types from sampler subcrate
#[cfg(feature = "sampler")]
pub use tutti_sampler::TimeStretchParams;

// Recording types from sampler subcrate
#[cfg(feature = "sampler")]
pub use tutti_sampler::{
    PunchEvent, QuantizeSettings, QuantizeSettingsBuilder, RecordedData, RecordingBuffer,
    RecordingConfig, RecordingConfigBuilder, RecordingMode, RecordingSession, RecordingSource,
    RecordingState as SamplerRecordingState, XRunEvent, XRunType,
};

// Synth subsystem (building blocks for synthesis)
#[cfg(feature = "synth")]
pub use tutti_synth as synth;

// SoundFont synthesis
#[cfg(feature = "soundfont")]
pub use tutti_synth::{
    SoundFont, SoundFontHandle, SoundFontSystem, SoundFontUnit, SynthesizerSettings,
};

// DSP nodes
pub use tutti_dsp as dsp_nodes;

// LFO is always available
pub use tutti_dsp::{LfoMode, LfoNode, LfoShape};

// Dynamics + spatial (compressors, gates, VBAP, binaural)
#[cfg(feature = "dsp")]
pub use tutti_dsp::{
    BinauralPannerNode, ChannelLayout, SidechainCompressor, SidechainGate, SpatialPannerNode,
    StereoSidechainCompressor, StereoSidechainGate,
};

// Analysis tools (optional)
#[cfg(feature = "analysis")]
pub use tutti_analysis as analysis;

#[cfg(feature = "analysis")]
pub use tutti_analysis::{
    AnalysisHandle, CorrelationMeter, DetectionMethod, LiveAnalysisState, MultiResolutionSummary,
    PitchDetector, PitchResult, StereoAnalysis, StereoWaveformSummary, Transient,
    TransientDetector, WaveformBlock, WaveformSummary,
};

// Export (optional)
#[cfg(feature = "export")]
pub use tutti_export as export;

#[cfg(feature = "export")]
pub use tutti_export::{
    AudioFormat, ExportBuilder, ExportConfig, ExportContext, ExportHandle, ExportOptions,
    ExportStatus, NormalizationMode,
};

// Export timeline (for advanced export scenarios)
#[cfg(feature = "export")]
pub use tutti_core::ExportTimeline;

// Plugin hosting
#[cfg(feature = "plugin")]
pub use tutti_plugin as plugin;

#[cfg(feature = "plugin")]
pub use tutti_plugin::{
    register_all_system_plugins, register_plugin, register_plugin_directory, BridgeConfig,
    ParameterFlags, ParameterInfo, PluginClient, PluginHandle, PluginMetadata,
};

// Neural audio
#[cfg(feature = "neural")]
pub use tutti_neural as neural;

#[cfg(feature = "neural")]
pub use tutti_neural::{NeuralHandle, NeuralSystem, NeuralSystemBuilder};

// Neural types from tutti-core
#[cfg(feature = "neural")]
pub use tutti_core::{
    BackendCapabilities, BackendFactory, InferenceBackend, InferenceConfig, InferenceError,
    NeuralModelId,
};

// Automation (optional)
#[cfg(feature = "automation")]
pub use tutti_automation as automation;

#[cfg(feature = "automation")]
pub use tutti_automation::{
    AutomationClip, AutomationEnvelope, AutomationLane, AutomationPoint, AutomationState,
    CurveType, LiveAutomationLane,
};

// Burn ML backend (optional)
#[cfg(feature = "burn")]
pub use tutti_burn;

/// Full FunDSP prelude - oscillators, filters, effects, and more.
///
/// Includes:
/// - **Oscillators**: `sine_hz`, `saw_hz`, `square_hz`, `triangle_hz`, `pulse`, `organ`, `hammond`, etc.
/// - **Filters**: `lowpass_hz`, `highpass_hz`, `bandpass_hz`, `notch_hz`, `peak_hz`, `bell_hz`,
///   `moog_hz`, `resonator_hz`, `butterpass_hz`, `allpass_hz`, `lowshelf_hz`, `highshelf_hz`,
///   `lr_lowpass_hz`, `lr_highpass_hz`, `lr_crossover_hz` (Linkwitz-Riley), etc.
/// - **Effects**: `reverb_stereo`, `chorus`, `flanger`, `phaser`, `delay`, `feedback`, `limiter_stereo`, etc.
/// - **Noise**: `white`, `pink`, `brown`, `noise`
/// - **Envelopes**: `adsr_live`, `envelope`, `lfo`, `follow`, `afollow`
/// - **Spatial**: `pan`, `panner`, `rotate`
/// - **Dynamics**: `limiter`, `clip`, `shape`
/// - **Utilities**: `pass`, `sink`, `zero`, `dc`, `constant`, `split`, `join`, etc.
/// - **Graph operators**: `>>` (pipe), `&` (bus), `^` (branch), `|` (stack)
///
/// See FunDSP documentation for full list: <https://docs.rs/fundsp>
pub mod dsp {
    pub use tutti_core::dsp::*;
}

mod builder;
pub mod builders;
mod engine;

pub use builder::TuttiEngineBuilder;
pub use engine::TuttiEngine;

// Re-export builder types for convenience
#[cfg(feature = "neural")]
pub use builders::NeuralEffectBuilder;
#[cfg(all(feature = "neural", feature = "midi"))]
pub use builders::NeuralSynthBuilder;
#[cfg(feature = "plugin")]
pub use builders::PluginBuilder;
#[cfg(feature = "sampler")]
pub use builders::SampleBuilder;
#[cfg(feature = "soundfont")]
pub use builders::Sf2Builder;

// SynthHandle for fluent synth creation
#[cfg(all(feature = "synth", feature = "midi"))]
pub use tutti_synth::SynthHandle;

/// Convenience prelude for common imports
pub mod prelude {
    // Main engine
    pub use crate::{TuttiEngine, TuttiEngineBuilder};

    // Essential types
    pub use crate::core::{AudioUnit, BufferMut, BufferRef, SignalFrame};

    // FunDSP toolkit
    pub use crate::dsp::*;

    // Transport
    pub use crate::core::TransportManager;

    // Node parameters (for create() calls)
    pub use crate::core::{NodeParamValue, NodeParams, Params};

    // Re-export macros from tutti-core
    pub use tutti_core::{chain, mix, params, split};

    // MIDI (optional)
    #[cfg(feature = "midi")]
    pub use crate::midi::{MidiEvent, MidiHandle, MidiSystem, Note};

    // Sampler (optional)
    #[cfg(feature = "sampler")]
    pub use crate::sampler::{SamplerHandle, SamplerSystem, SamplerUnit};

    // Neural (optional)
    #[cfg(feature = "neural")]
    pub use crate::neural::{NeuralHandle, NeuralSystem};
    #[cfg(feature = "neural")]
    pub use tutti_core::NeuralModelId;

    // Export (optional)
    #[cfg(feature = "export")]
    pub use crate::export::{AudioFormat, ExportBuilder, NormalizationMode};

    // Analysis (optional)
    #[cfg(feature = "analysis")]
    pub use crate::analysis::{PitchDetector, TransientDetector};

    // Automation (optional)
    #[cfg(feature = "automation")]
    pub use crate::automation::{AutomationEnvelope, AutomationLane, AutomationPoint, CurveType};
}
