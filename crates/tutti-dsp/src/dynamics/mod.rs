//! Compressors and gates with external sidechain support.

mod utils;

mod compressor;
mod gate;
mod stereo_compressor;
mod stereo_gate;

pub use compressor::{SidechainCompressor, SidechainCompressorBuilder};
pub use gate::{SidechainGate, SidechainGateBuilder};
pub use stereo_compressor::StereoSidechainCompressor;
pub use stereo_gate::StereoSidechainGate;
