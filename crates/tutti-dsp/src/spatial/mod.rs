//! Spatial Audio Processing
//!
//! This module provides spatial audio capabilities for multichannel and immersive audio.
//!
//! ## Features
//!
//! - **VBAP Panning** - Vector Base Amplitude Panning for surround sound (5.1, 7.1, Atmos)
//! - **Binaural Panning** - Simple ITD/ILD model for headphone 3D audio
//! - **Channel Layouts** - Standard speaker configurations (stereo, 5.1, 7.1, 7.1.4)
//! - **AudioUnit Nodes** - FunDSP Net integration for graph-based spatial processing
//!
//! ## Integration with FunDSP Net
//!
//! Spatial panners can be added to the audio graph as AudioUnit nodes:
//!
//! ```ignore
//! use tutti::net::TuttiNet;
//! use tutti::spatial::{SpatialPannerNode, BinauralPannerNode};
//!
//! // Create a 5.1 surround panner node
//! let panner = SpatialPannerNode::surround_5_1()?;
//! panner.set_position(45.0, 0.0); // Front-left at ear level
//!
//! // Add to Net (stereo input → 6 channel output)
//! let node_id = net.add(Box::new(panner));
//! net.connect(synth_id, 0, node_id, 0);
//! net.connect(synth_id, 1, node_id, 1);
//! net.pipe_output(node_id);
//! net.commit();
//!
//! // Or use binaural for headphones (stereo → stereo with 3D cues)
//! let binaural = BinauralPannerNode::new(48000.0);
//! binaural.set_position(90.0, 0.0); // Hard left
//! ```
//!
//! ## Comparison: FunDSP pan() vs Spatial Panners
//!
//! | Feature | FunDSP `pan()` | `SpatialPannerNode` | `BinauralPannerNode` |
//! |---------|----------------|---------------------|----------------------|
//! | Output | Stereo (2ch) | Multichannel (2-12ch) | Stereo (2ch) |
//! | Panning | Left/Right only | Full 3D (azimuth/elevation) | Full 3D |
//! | Use case | Simple stereo | Surround sound systems | Headphones |
//! | Algorithm | Equal power | VBAP | ITD/ILD |
//!
//! ## Note on Stereo vs Spatial
//!
//! - **Simple stereo**: Use FunDSP's `pan()` for basic left/right panning
//! - **Surround sound**: Use `SpatialPannerNode` for 5.1/7.1/Atmos speaker setups
//! - **Headphones 3D**: Use `BinauralPannerNode` for immersive headphone audio

// Export common types
pub mod types;
pub use types::ChannelLayout;

// Internal utilities
mod utils;

// Spatial panner implementations (internal)
mod binaural_panner;
mod nodes;
mod vbap_panner;

// Only export AudioUnit nodes - raw panners are internal
pub use nodes::{BinauralPannerNode, SpatialPannerNode};

// Re-export vbap types for advanced usage (custom speaker configurations)
// Note: Most users should use SpatialPannerNode's preset constructors instead
#[allow(unused_imports)]
pub use vbap::{SpeakerConfig, SpeakerConfigBuilder, VBAPanner};
