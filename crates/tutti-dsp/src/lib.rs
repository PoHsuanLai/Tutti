//! # Tutti DSP
//!
//! Real-time DSP building blocks for the Tutti audio engine.
//!
//! This crate provides AudioUnit nodes for:
//! - **LFO** - Low frequency oscillators with multiple waveforms and beat-sync
//! - **Dynamics** - Compressors and gates with sidechain support
//! - **Spatial Audio** - VBAP and binaural panning for immersive audio
//!
//! All nodes are RT-safe and use lock-free atomics for parameter control.
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
//! ### Sidechain Compressor
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

mod dynamics;
mod lfo;
mod spatial;

pub use dynamics::{
    SidechainCompressor, SidechainCompressorBuilder, SidechainGate, SidechainGateBuilder,
    StereoSidechainCompressor, StereoSidechainGate,
};
pub use lfo::{LfoMode, LfoNode, LfoShape};
pub use spatial::{BinauralPannerNode, ChannelLayout, SpatialPannerNode};

// Fluent API handles
mod handles;
pub use handles::{DspHandle, SidechainHandle, SpatialHandle};
