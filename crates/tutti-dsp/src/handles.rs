//! Fluent API handles for DSP node registration
//!
//! This module provides ergonomic handles for adding DSP nodes to the engine's node registry.
//! Nodes are registered once and can be instantiated multiple times with different parameters.
//!
//! # Example
//! ```ignore
//! use tutti::prelude::*;
//! use tutti::dsp_nodes::{LfoShape, ChannelLayout};
//!
//! let engine = TuttiEngine::builder().build()?;
//!
//! // Register DSP node types via handles
//! let dsp = engine.dsp();
//! dsp.lfo("bass_lfo", LfoShape::Sine, 0.5);
//! dsp.sidechain().compressor("comp", -20.0, 4.0, 0.001, 0.05);
//! dsp.spatial().vbap("panner", ChannelLayout::stereo());
//! ```

use crate::{
    AudioUnit, BinauralPannerNode, ChannelLayout, LfoNode, LfoShape, SidechainCompressor,
    SidechainGate, SpatialPannerNode, StereoSidechainCompressor, StereoSidechainGate,
};
use tutti_core::{get_param_or, NodeRegistry};

/// Main DSP handle for registering DSP nodes
///
/// Provides methods for adding LFO and grouped handles for dynamics and spatial audio nodes.
pub struct DspHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

impl<'a> DspHandle<'a> {
    /// Create a new DSP handle
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    /// Remove a registered DSP node type
    pub fn remove(&self, name: &str) -> bool {
        self.registry.unregister(name)
    }

    /// Check if a DSP node type is registered
    pub fn has(&self, name: &str) -> bool {
        self.registry.has_type(name)
    }

    /// List all registered DSP node type names
    pub fn list(&self) -> Vec<String> {
        self.registry.list_types()
    }

    /// Register an LFO (Low Frequency Oscillator) node
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `shape` - Waveform shape (Sine, Triangle, Saw, Square, Random)
    /// * `frequency` - Default frequency in Hz
    ///
    /// # Instance Parameters
    /// - `frequency` (f32) - Override frequency
    /// - `depth` (f32) - Modulation depth (0.0 to 1.0)
    /// - `phase_offset` (f32) - Phase offset (0.0 to 1.0)
    pub fn lfo(&self, name: impl Into<String>, shape: LfoShape, frequency: f32) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let freq = get_param_or(params, "frequency", frequency);
            let mut lfo = LfoNode::new(shape, freq);
            AudioUnit::set_sample_rate(&mut lfo, sample_rate);

            if let Some(depth) = params.get("depth").and_then(|v| v.as_f32()) {
                lfo.set_depth(depth);
            }
            if let Some(phase) = params.get("phase_offset").and_then(|v| v.as_f32()) {
                lfo.set_phase_offset(phase);
            }

            Ok(Box::new(lfo) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Get sidechain dynamics handle (compressor, gate)
    pub fn sidechain(&self) -> SidechainHandle<'a> {
        SidechainHandle::new(self.registry, self.sample_rate)
    }

    /// Get spatial audio handle (VBAP, binaural)
    pub fn spatial(&self) -> SpatialHandle<'a> {
        SpatialHandle::new(self.registry, self.sample_rate)
    }
}

/// Sidechain dynamics handle for compressors and gates
pub struct SidechainHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

impl<'a> SidechainHandle<'a> {
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    /// Register a sidechain compressor (mono)
    ///
    /// Inputs: Channel 0 = audio, Channel 1 = sidechain
    pub fn compressor(
        &self,
        name: impl Into<String>,
        threshold_db: f32,
        ratio: f32,
        attack_sec: f32,
        release_sec: f32,
    ) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |_params| {
            let mut comp = SidechainCompressor::new(threshold_db, ratio, attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut comp, sample_rate);
            Ok(Box::new(comp) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register a sidechain gate (mono)
    ///
    /// Inputs: Channel 0 = audio, Channel 1 = sidechain
    pub fn gate(
        &self,
        name: impl Into<String>,
        threshold_db: f32,
        attack_sec: f32,
        hold_sec: f32,
        release_sec: f32,
    ) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |_params| {
            let mut gate = SidechainGate::new(threshold_db, attack_sec, hold_sec, release_sec);
            AudioUnit::set_sample_rate(&mut gate, sample_rate);
            Ok(Box::new(gate) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register a stereo sidechain compressor
    ///
    /// Inputs: Channels 0-1 = stereo audio, Channels 2-3 = stereo sidechain
    pub fn stereo_compressor(
        &self,
        name: impl Into<String>,
        threshold_db: f32,
        ratio: f32,
        attack_sec: f32,
        release_sec: f32,
    ) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |_params| {
            let mut comp =
                StereoSidechainCompressor::new(threshold_db, ratio, attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut comp, sample_rate);
            Ok(Box::new(comp) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register a stereo sidechain gate
    ///
    /// Inputs: Channels 0-1 = stereo audio, Channels 2-3 = stereo sidechain
    pub fn stereo_gate(
        &self,
        name: impl Into<String>,
        threshold_db: f32,
        attack_sec: f32,
        hold_sec: f32,
        release_sec: f32,
    ) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |_params| {
            let mut gate =
                StereoSidechainGate::new(threshold_db, attack_sec, hold_sec, release_sec);
            AudioUnit::set_sample_rate(&mut gate, sample_rate);
            Ok(Box::new(gate) as Box<dyn AudioUnit>)
        });

        self
    }
}

/// Spatial audio handle for VBAP and binaural panners
pub struct SpatialHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

impl<'a> SpatialHandle<'a> {
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    /// Register a VBAP spatial panner
    ///
    /// Instance parameters: azimuth, elevation, spread
    pub fn vbap(&self, name: impl Into<String>, layout: ChannelLayout) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let mut panner = match layout {
                ChannelLayout {
                    left: 0,
                    right: 1,
                    center: None,
                    lfe: None,
                    surround_left: None,
                    surround_right: None,
                    rear_left: None,
                    rear_right: None,
                    height_front_left: None,
                    height_front_right: None,
                    height_rear_left: None,
                    height_rear_right: None,
                } => SpatialPannerNode::stereo().map_err(|e| {
                    tutti_core::NodeRegistryError::ConstructionFailed(e.to_string())
                })?,
                _ if layout.surround_left.is_some() && layout.surround_right.is_some() => {
                    if layout.height_front_left.is_some() {
                        SpatialPannerNode::atmos_7_1_4().map_err(|e| {
                            tutti_core::NodeRegistryError::ConstructionFailed(e.to_string())
                        })?
                    } else if layout.rear_left.is_some() {
                        SpatialPannerNode::surround_7_1().map_err(|e| {
                            tutti_core::NodeRegistryError::ConstructionFailed(e.to_string())
                        })?
                    } else {
                        SpatialPannerNode::surround_5_1().map_err(|e| {
                            tutti_core::NodeRegistryError::ConstructionFailed(e.to_string())
                        })?
                    }
                }
                _ => SpatialPannerNode::stereo().map_err(|e| {
                    tutti_core::NodeRegistryError::ConstructionFailed(e.to_string())
                })?,
            };

            AudioUnit::set_sample_rate(&mut panner, sample_rate);

            if let Some(azimuth) = params.get("azimuth").and_then(|v| v.as_f32()) {
                if let Some(elevation) = params.get("elevation").and_then(|v| v.as_f32()) {
                    panner.set_position(azimuth, elevation);
                }
            }
            if let Some(spread) = params.get("spread").and_then(|v| v.as_f32()) {
                panner.set_spread(spread);
            }

            Ok(Box::new(panner) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register a binaural panner (ITD/ILD model for headphones)
    ///
    /// Instance parameters: azimuth, elevation
    pub fn binaural(&self, name: impl Into<String>) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let panner = BinauralPannerNode::new(sample_rate as f32);

            if let Some(azimuth) = params.get("azimuth").and_then(|v| v.as_f32()) {
                if let Some(elevation) = params.get("elevation").and_then(|v| v.as_f32()) {
                    panner.set_position(azimuth, elevation);
                }
            }

            Ok(Box::new(panner) as Box<dyn AudioUnit>)
        });

        self
    }
}
