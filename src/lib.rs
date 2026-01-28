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
//! // Create engine (capabilities depend on enabled features)
//! let engine = TuttiEngine::builder()
//!     .sample_rate(44100.0)
//!     .build()?;
//!
//! // Build audio graph
//! engine.graph(|net| {
//!     let osc = net.add(Box::new(sine_hz(440.0)));
//!     net.pipe_output(osc);
//! });
//!
//! // Control transport
//! engine.transport().play();
//! ```
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
    CpuMeter,
    CpuMetrics,

    // Error
    Error,
    EventId,
    Fade,
    // Metering (includes LUFS!)
    MeteringManager,
    Metronome,
    MetronomeMode,
    MotionState,
    NetBackend,

    NodeId,
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
    TransportManager,
    TuttiNet,
    BBT,
};

// Atomic types
pub use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

// Sample types
pub use tutti_core::{Sample, F32, F64};

// MIDI subsystem
#[cfg(feature = "midi")]
pub use tutti_midi as midi;

#[cfg(feature = "midi")]
pub use tutti_midi::{MidiEvent, MidiSystem, MidiSystemBuilder, PortInfo, RawMidiEvent};

// Sampler subsystem
#[cfg(feature = "sampler")]
pub use tutti_sampler as sampler;

#[cfg(feature = "sampler")]
pub use tutti_sampler::{
    AudioInput, AudioInputBackend, SamplerSystem, SamplerSystemBuilder, SamplerUnit,
    StreamingSamplerUnit, TimeStretchUnit,
};

// Time stretch types from sampler subcrate
#[cfg(feature = "sampler")]
pub use tutti_sampler::time_stretch::TimeStretchParams;

// DSP nodes
pub use tutti_dsp as dsp_nodes;

pub use tutti_dsp::{
    ChannelLayout, EnvelopeFollowerNode, EnvelopeMode, LfoMode, LfoNode, LfoShape,
    SidechainCompressor, SidechainGate, StereoSidechainCompressor, StereoSidechainGate,
};

#[cfg(feature = "spatial-audio")]
pub use tutti_dsp::{BinauralPanner, BinauralPannerNode, SpatialPanner, SpatialPannerNode};

// Analysis tools
#[cfg(feature = "analysis")]
pub use tutti_analysis as analysis;

#[cfg(feature = "analysis")]
pub use tutti_analysis::{
    CorrelationMeter, PitchDetector, PitchResult, StereoAnalysis, Transient, TransientDetector,
};

// Export
#[cfg(feature = "export")]
pub use tutti_export as export;

#[cfg(feature = "export")]
pub use tutti_export::{AudioFormat, ExportOptions, OfflineRenderer};

// Plugin hosting
#[cfg(feature = "plugin")]
pub use tutti_plugin as plugin;

#[cfg(feature = "plugin")]
pub use tutti_plugin::{BridgeConfig, PluginClient};

// Neural audio
#[cfg(feature = "neural")]
pub use tutti_neural as neural;

#[cfg(feature = "neural")]
pub use tutti_neural::{NeuralSystem, NeuralSystemBuilder};

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
    pub use fundsp::prelude::*;
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

    // MIDI
    #[cfg(feature = "midi")]
    pub use crate::midi::{MidiEvent, MidiSystem};

    // Sampler
    #[cfg(feature = "sampler")]
    pub use crate::sampler::{SamplerSystem, SamplerUnit};

    // Neural
    #[cfg(feature = "neural")]
    pub use crate::neural::NeuralSystem;
}
