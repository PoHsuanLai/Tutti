//! RT-safe DSP building blocks: LFO, dynamics (compressors/gates with sidechain),
//! and spatial audio (VBAP/binaural). All nodes use lock-free atomics for parameter control.

mod error;
pub use error::{Error, Result};

mod lfo;
pub use lfo::{LfoMode, LfoNode, LfoShape};

#[cfg(feature = "dynamics")]
mod dynamics;
#[cfg(feature = "dynamics")]
pub use dynamics::{
    SidechainCompressor, SidechainCompressorBuilder, SidechainGate, SidechainGateBuilder,
    StereoSidechainCompressor, StereoSidechainGate,
};

#[cfg(feature = "spatial")]
mod spatial;
#[cfg(feature = "spatial")]
pub use spatial::{BinauralPannerNode, ChannelLayout, SpatialPannerNode};

mod handles;
pub use handles::DspHandle;
#[cfg(feature = "dynamics")]
pub use handles::SidechainHandle;
#[cfg(feature = "spatial")]
pub use handles::SpatialHandle;
