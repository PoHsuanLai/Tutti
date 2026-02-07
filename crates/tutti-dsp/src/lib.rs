//! # Tutti DSP
//!
//! Real-time DSP building blocks for the Tutti audio engine.
//!
//! This crate provides AudioUnit nodes for:
//! - **LFO** - Low frequency oscillators with multiple waveforms and beat-sync
//! - **Dynamics** - Compressors and gates with sidechain support (requires `dynamics` feature)
//! - **Spatial Audio** - VBAP and binaural panning for immersive audio (requires `spatial` feature)
//!
//! All nodes are RT-safe and use lock-free atomics for parameter control.
//!
//! ## Features
//!
//! - `default` - Just LFO (no external dependencies)
//! - `dynamics` - Compressors and gates with sidechain support
//! - `spatial` - VBAP and binaural panning (adds `vbap` dependency)
//! - `full` - All features enabled
//!
//! ## Note on Envelope Following
//!
//! For envelope detection, use the `afollow` function from `tutti::dsp`:
//!
//! ```ignore
//! use tutti::dsp::afollow;
//!
//! // Envelope follower with 10ms attack, 100ms release
//! let env = afollow(0.01, 0.1);
//! ```
//!
//! This crate focuses on higher-level processors not available in FunDSP.
//!
//! ## Examples
//!
//! ### LFO (Low Frequency Oscillator)
//!
//! ```ignore
//! use tutti_dsp::{LfoNode, LfoShape};
//! use tutti_core::AudioUnit;
//!
//! // Create an LFO
//! let mut lfo = LfoNode::new(LfoShape::Sine, 2.0);
//! lfo.set_sample_rate(44100.0);
//! lfo.set_depth(0.8);
//! lfo.set_phase_offset(0.25);
//!
//! // Process audio
//! let mut output = [0.0f32; 1];
//! lfo.tick(&[], &mut output);
//! ```
//!
//! ### Sidechain Compressor (requires `dynamics` feature)
//!
//! ```ignore
//! use tutti_dsp::SidechainCompressor;
//! use tutti_core::AudioUnit;
//!
//! let comp = SidechainCompressor::builder()
//!     .threshold_db(-20.0)
//!     .ratio(4.0)
//!     .attack_seconds(0.001)
//!     .release_seconds(0.05)
//!     .build();
//! ```

// Re-export AudioUnit trait from tutti-core
pub use tutti_core::AudioUnit;

// Error types
mod error;
pub use error::{Error, Result};

// LFO is always available (no external deps)
mod lfo;
pub use lfo::{LfoMode, LfoNode, LfoShape};

// Dynamics: compressors and gates (no external deps)
#[cfg(feature = "dynamics")]
mod dynamics;
#[cfg(feature = "dynamics")]
pub use dynamics::{
    SidechainCompressor, SidechainCompressorBuilder, SidechainGate, SidechainGateBuilder,
    StereoSidechainCompressor, StereoSidechainGate,
};

// Spatial audio: VBAP and binaural (requires vbap crate)
#[cfg(feature = "spatial")]
mod spatial;
#[cfg(feature = "spatial")]
pub use spatial::{BinauralPannerNode, ChannelLayout, SpatialPannerNode};

// Fluent API handles
mod handles;
pub use handles::DspHandle;
#[cfg(feature = "dynamics")]
pub use handles::SidechainHandle;
#[cfg(feature = "spatial")]
pub use handles::SpatialHandle;
