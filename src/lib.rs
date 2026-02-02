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
//! // Create tokio runtime for plugin loading (optional)
//! let runtime = tokio::runtime::Runtime::new()?;
//!
//! // Create engine (subsystems enabled via Cargo features)
//! let engine = TuttiEngine::builder()
//!     .sample_rate(44100.0)
//!     .plugin_runtime(runtime.handle().clone())  // Optional: for plugin loading
//!     .build()?;
//!
//! // Load nodes once (explicit format methods = compile-time type safety)
//! engine.load_mpk("my_synth", "model.mpk")?;      // Neural model
//! engine.load_vst3("reverb", "plugin.vst3")?;     // VST3 plugin
//!
//! // Add custom DSP nodes programmatically
//! engine.add_node("my_filter", |params| {
//!     let cutoff = params.get("cutoff")?.as_f32().unwrap_or(1000.0);
//!     Ok(Box::new(lowpass_hz(cutoff)))
//! })?;
//!
//! // Instantiate nodes (creates instances and adds to graph)
//! let synth = engine.instance("my_synth", &params! {})?;
//! let filter = engine.instance("my_filter", &params! { "cutoff" => 2000.0 })?;
//! let reverb = engine.instance("reverb", &params! { "room_size" => 0.9 })?;
//!
//! // Build audio graph with node IDs
//! engine.graph(|net| {
//!     chain!(net, synth, filter, reverb => output);
//! });
//!
//! // Control transport
//! engine.transport().play();
//! ```
//!
//! ## Built-in Audio Nodes
//!
//! Tutti provides many AudioUnit implementations out of the box:
//!
//! - **Synths**: `PolySynth` (waveform synth), `SoundFontUnit` (sample-based)
//! - **Samplers**: `SamplerUnit`, `StreamingSamplerUnit`, `TimeStretchUnit`
//! - **DSP**: `LfoNode`, `EnvelopeFollowerNode`, sidechain dynamics
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

// Core types
pub use tutti_core::{
    get_param,
    get_param_or,

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
    Connection,
    CpuMeter,
    CpuMetrics,

    // Error
    Error,
    EventId,
    Fade,
    // Metering (includes LUFS!)
    MeteringManager,
    MetronomeHandle,
    Metronome,
    MetronomeMode,
    MotionState,
    NetBackend,

    NodeConstructor,
    NodeId,
    // Node introspection
    NodeInfo,
    NodeParamValue,
    NodeParams,
    // Node registry
    NodeRegistry,
    NodeRegistryError,
    PdcDelayUnit,

    // PDC
    PdcManager,
    PdcState,
    ReplayMode,

    Result,
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
    TuttiNet,
    BBT,
};

// Atomic types
pub use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

// Sample types
pub use tutti_core::{Sample, F32, F64};

// MIDI I/O subsystem
#[cfg(feature = "midi")]
pub use tutti_midi_io as midi;

#[cfg(feature = "midi")]
pub use tutti_midi_io::{MidiEvent, MidiSystem, MidiSystemBuilder, MidiHandle, PortInfo, RawMidiEvent};

#[cfg(feature = "midi")]
pub use tutti_core::{AsMidiAudioUnit, MidiAudioUnit, MidiRegistry};

// Sampler subsystem (optional)
#[cfg(feature = "sampler")]
pub use tutti_sampler as sampler;

#[cfg(feature = "sampler")]
pub use tutti_sampler::{
    AudioInput, AudioInputBackend, SamplerHandle, SamplerSystem, SamplerSystemBuilder, SamplerUnit,
    StreamingSamplerUnit, TimeStretchUnit,
};

// Time stretch types from sampler subcrate
#[cfg(feature = "sampler")]
pub use tutti_sampler::time_stretch::TimeStretchParams;

// Synth subsystem
#[cfg(feature = "synth")]
pub use tutti_synth as synth;

#[cfg(feature = "synth")]
pub use tutti_synth::{Envelope, PolySynth, Waveform};

#[cfg(feature = "soundfont")]
pub use tutti_synth::{SoundFontSystem, SoundFontSynth, SoundFontUnit};

// DSP nodes
pub use tutti_dsp as dsp_nodes;

pub use tutti_dsp::{
    ChannelLayout, EnvelopeFollowerNode, EnvelopeMode, LfoMode, LfoNode, LfoShape,
    SidechainCompressor, SidechainGate, StereoSidechainCompressor, StereoSidechainGate,
};

// Spatial audio (always included) - only AudioUnit nodes
pub use tutti_dsp::{BinauralPannerNode, SpatialPannerNode};

// Analysis tools (optional)
#[cfg(feature = "analysis")]
pub use tutti_analysis as analysis;

#[cfg(feature = "analysis")]
pub use tutti_analysis::{
    CorrelationMeter, PitchDetector, PitchResult, StereoAnalysis, Transient, TransientDetector,
};

// Export (optional)
#[cfg(feature = "export")]
pub use tutti_export as export;

#[cfg(feature = "export")]
pub use tutti_export::{AudioFormat, ExportBuilder, ExportOptions};

// Plugin hosting
#[cfg(feature = "plugin")]
pub use tutti_plugin as plugin;

#[cfg(feature = "plugin")]
pub use tutti_plugin::{
    register_all_system_plugins, register_plugin, register_plugin_directory, BridgeConfig,
    PluginClient,
};

// Neural audio
#[cfg(feature = "neural")]
pub use tutti_neural as neural;

#[cfg(feature = "neural")]
pub use tutti_neural::{
    register_all_neural_models, register_neural_directory, register_neural_effects,
    register_neural_model, register_neural_synth_models, NeuralHandle, NeuralModelMetadata,
    NeuralModelType, NeuralSystem, NeuralSystemBuilder,
};

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
mod engine;

pub use builder::TuttiEngineBuilder;
pub use engine::TuttiEngine;

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

    // Node parameters (for instance() calls)
    pub use crate::core::{
        get_param, get_param_or, NodeParamValue, NodeParams,
    };

    // Re-export macros from tutti-core
    pub use tutti_core::{chain, mix, params, split};

    // MIDI (optional)
    #[cfg(feature = "midi")]
    pub use crate::midi::{MidiEvent, MidiSystem, MidiHandle};

    // Sampler (optional)
    #[cfg(feature = "sampler")]
    pub use crate::sampler::{SamplerHandle, SamplerSystem, SamplerUnit};

    // Neural (optional)
    #[cfg(feature = "neural")]
    pub use crate::neural::{NeuralHandle, NeuralSystem};

    // Export (optional)
    #[cfg(feature = "export")]
    pub use crate::export::{AudioFormat, ExportBuilder};

    // Analysis (optional)
    #[cfg(feature = "analysis")]
    pub use crate::analysis::{TransientDetector, PitchDetector};
}
