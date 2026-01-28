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
    // Audio graph
    AudioUnit, BufferRef, BufferMut, SignalFrame, Shared,
    TuttiNet, NodeId, Source, NetBackend,

    // Transport
    TransportManager, TempoMap, TimeSignature, BBT, MotionState,
    Metronome, MetronomeMode, TransportClock,

    // Metering (includes LUFS!)
    MeteringManager, AtomicAmplitude, AtomicStereoAnalysis,
    StereoAnalysisSnapshot, CpuMeter, CpuMetrics,

    // PDC
    PdcManager, PdcState, PdcDelayUnit,

    // Lock-free primitives
    AtomicFloat, AtomicDouble, AtomicFlag,

    // Sequencer
    Sequencer, EventId, Fade, ReplayMode,

    // Error
    Error, Result,
};

// Atomic types
pub use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, AtomicUsize, Ordering};

// Sample types
pub use tutti_core::{Sample, F32, F64};

// MIDI subsystem
#[cfg(feature = "midi")]
pub use tutti_midi as midi;

#[cfg(feature = "midi")]
pub use tutti_midi::{
    MidiSystem, MidiSystemBuilder,
    MidiEvent, RawMidiEvent,
    PortInfo,
};

// Sampler subsystem
#[cfg(feature = "sampler")]
pub use tutti_sampler as sampler;

#[cfg(feature = "sampler")]
pub use tutti_sampler::{
    SamplerSystem, SamplerSystemBuilder,
    SamplerUnit, StreamingSamplerUnit,
    TimeStretchUnit, TimeStretchParams,
    AudioInput, AudioInputBackend,
};

// DSP nodes
pub use tutti_dsp as dsp_nodes;

pub use tutti_dsp::{
    LfoNode, LfoShape, LfoMode,
    EnvelopeFollowerNode, EnvelopeMode,
    SidechainCompressor, StereoSidechainCompressor,
    SidechainGate, StereoSidechainGate,
    ChannelLayout,
};

#[cfg(feature = "spatial-audio")]
pub use tutti_dsp::{
    SpatialPannerNode, BinauralPannerNode,
    SpatialPanner, BinauralPanner,
};

// Analysis tools
#[cfg(feature = "analysis")]
pub use tutti_analysis as analysis;

#[cfg(feature = "analysis")]
pub use tutti_analysis::{
    TransientDetector, PitchDetector, CorrelationMeter,
    Transient, PitchResult, StereoAnalysis,
};

// Export
#[cfg(feature = "export")]
pub use tutti_export as export;

#[cfg(feature = "export")]
pub use tutti_export::{
    OfflineRenderer, ExportOptions, AudioFormat,
};

// Plugin hosting
#[cfg(feature = "plugin")]
pub use tutti_plugin as plugin;

#[cfg(feature = "plugin")]
pub use tutti_plugin::{
    PluginClient, BridgeConfig,
};

// Neural audio
#[cfg(feature = "neural")]
pub use tutti_neural as neural;

#[cfg(feature = "neural")]
pub use tutti_neural::{
    NeuralSystem, NeuralSystemBuilder,
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
    pub use fundsp::prelude::*;
}

mod engine;
mod builder;

pub use engine::TuttiEngine;
pub use builder::TuttiEngineBuilder;

/// Convenience prelude for common imports
pub mod prelude {
    // Main engine
    pub use crate::{TuttiEngine, TuttiEngineBuilder};

    // Essential types
    pub use crate::core::{AudioUnit, BufferRef, BufferMut, SignalFrame};

    // FunDSP toolkit
    pub use crate::dsp::*;

    // Transport
    pub use crate::core::TransportManager;

    // MIDI
    #[cfg(feature = "midi")]
    pub use crate::midi::{MidiSystem, MidiEvent};

    // Sampler
    #[cfg(feature = "sampler")]
    pub use crate::sampler::{SamplerSystem, SamplerUnit};

    // Neural
    #[cfg(feature = "neural")]
    pub use crate::neural::NeuralSystem;
}
