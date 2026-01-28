//! Time-stretching and pitch-shifting for sample playback.
//!
//! Provides real-time time-stretching and pitch-shifting capabilities using
//! phase vocoder techniques. Can wrap any AudioUnit to add time/pitch manipulation.
//!
//! # Example
//!
//! ```ignore
//! use tutti_sampler::{SamplerUnit, TimeStretchUnit};
//! use std::sync::Arc;
//!
//! // Create a sampler with a loaded audio file
//! let sampler = SamplerUnit::new(Arc::new(wave));
//!
//! // Wrap with time-stretch capability
//! let mut stretched = TimeStretchUnit::new(Box::new(sampler), 44100.0);
//!
//! // Slow down to half speed
//! stretched.set_stretch_factor(2.0);
//!
//! // Pitch up by one octave
//! stretched.set_pitch_cents(1200.0);
//! ```
//!
//! # Features
//!
//! - **Lock-free parameter updates**: Real-time control via atomic operations
//! - **High-quality phase vocoder**: Phase-locked algorithm for pitched content
//! - **Multiple FFT sizes**: Trade-off between latency and quality
//! - **Stereo processing**: Independent left/right channel processing

mod types;
mod phase_vocoder;
mod unit;

pub use types::{
    AtomicF32,
    TimeStretchParams,
    TimeStretchAlgorithm,
    FftSize,
};

pub use phase_vocoder::PhaseVocoderProcessor;
pub use unit::TimeStretchUnit;
