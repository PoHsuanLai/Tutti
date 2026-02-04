//! Dynamics processors with sidechain support.
//!
//! This module provides compressors and gates that use external sidechain
//! signals for level detection, enabling ducking, pumping, and other effects.
//!
//! ## Compressors
//!
//! - [`SidechainCompressor`] - Mono compressor with external sidechain
//! - [`StereoSidechainCompressor`] - Stereo linked compressor
//!
//! ## Gates
//!
//! - [`SidechainGate`] - Mono gate with external sidechain
//! - [`StereoSidechainGate`] - Stereo linked gate
//!
//! ## Example
//!
//! ```ignore
//! use tutti_dsp::SidechainCompressor;
//!
//! let comp = SidechainCompressor::builder()
//!     .threshold_db(-20.0)
//!     .ratio(4.0)
//!     .attack_seconds(0.001)
//!     .release_seconds(0.05)
//!     .build();
//! ```

mod utils;

mod compressor;
mod gate;
mod stereo_compressor;
mod stereo_gate;

pub use compressor::{SidechainCompressor, SidechainCompressorBuilder};
pub use gate::{SidechainGate, SidechainGateBuilder};
pub use stereo_compressor::StereoSidechainCompressor;
pub use stereo_gate::StereoSidechainGate;
