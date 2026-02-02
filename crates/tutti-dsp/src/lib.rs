//! # Tutti DSP
//!
//! Real-time DSP building blocks for the Tutti audio engine.
//!
//! This crate provides AudioUnit nodes for:
//! - **LFO** - Low frequency oscillators with multiple waveforms
//! - **Envelope Follower** - Peak and RMS envelope detection
//! - **Dynamics** - Compressors and gates with sidechain support
//! - **Spatial Audio** - VBAP and binaural panning for immersive audio
//!
//! All nodes are RT-safe and use lock-free atomics for parameter control.
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
//! ### Envelope Follower
//!
//! ```ignore
//! use tutti_dsp::{EnvelopeFollowerNode, EnvelopeMode};
//! use tutti_core::AudioUnit;
//!
//! // Peak envelope detection
//! let mut env = EnvelopeFollowerNode::new(0.001, 0.1);  // 1ms attack, 100ms release
//! env.set_sample_rate(44100.0);
//!
//! // Or RMS mode
//! let mut env_rms = EnvelopeFollowerNode::new_rms(0.001, 0.1, 10.0);  // 10ms window
//! ```
//!
//! ### Sidechain Compressor
//!
//! ```ignore
//! use tutti_dsp::SidechainCompressor;
//! use tutti_core::AudioUnit;
//!
//! let mut comp = SidechainCompressor::new(-20.0, 4.0, 0.001, 0.05);
//! comp.set_sample_rate(44100.0);
//!
//! // Process: audio input on channel 0, sidechain on channel 1
//! let input = [audio_sample, sidechain_sample];
//! let mut output = [0.0f32];
//! comp.tick(&input, &mut output);
//! ```

// Re-export AudioUnit trait from tutti-core
pub use tutti_core::AudioUnit;

// Error types
mod error;
pub use error::{Error, Result};

mod dynamics;
mod envelope_follower;
mod lfo;
mod spatial;

pub use dynamics::{
    SidechainCompressor, SidechainGate, StereoSidechainCompressor, StereoSidechainGate,
};
pub use envelope_follower::{EnvelopeFollowerNode, EnvelopeMode};
pub use lfo::{LfoMode, LfoNode, LfoShape};
pub use spatial::{
    BinauralPannerNode, ChannelLayout, SpatialPannerNode,
};

// Fluent API handles
mod handles;
pub use handles::{DspHandle, SidechainHandle, SpatialHandle};
