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
//! dsp.lfo("bass_lfo", LfoShape::Sine, 0.5)?;
//! dsp.sidechain().compressor("comp", -20.0, 4.0, 0.001, 0.05)?;
//! dsp.spatial().vbap("panner", ChannelLayout::Stereo)?;
//!
//! // Instantiate nodes with parameters
//! let lfo = engine.instance("bass_lfo", &params! { "depth" => 0.8 });
//! let comp = engine.instance("comp", &params! {});
//! ```

use crate::{
    AudioUnit, BinauralPannerNode, ChannelLayout, EnvelopeFollowerNode, LfoNode, LfoShape,
    SidechainCompressor, SidechainGate, SpatialPannerNode, StereoSidechainCompressor,
    StereoSidechainGate,
};
use tutti_core::{get_param_or, NodeRegistry};

/// Main DSP handle for registering DSP nodes
///
/// Provides methods for adding LFO, envelope follower, and grouped handles
/// for dynamics and spatial audio nodes.
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
    ///
    /// Returns true if the node was registered and removed, false otherwise.
    ///
    /// # Example
    /// ```ignore
    /// engine.dsp().lfo("bass_lfo", LfoShape::Sine, 0.5);
    /// engine.dsp().remove("bass_lfo"); // Returns true
    /// engine.dsp().remove("bass_lfo"); // Returns false (already removed)
    /// ```
    pub fn remove(&self, name: &str) -> bool {
        self.registry.unregister(name)
    }

    /// Check if a DSP node type is registered
    ///
    /// # Example
    /// ```ignore
    /// if engine.dsp().has("bass_lfo") {
    ///     println!("LFO is registered");
    /// }
    /// ```
    pub fn has(&self, name: &str) -> bool {
        self.registry.has_type(name)
    }

    /// List all registered DSP node type names
    ///
    /// # Example
    /// ```ignore
    /// for name in engine.dsp().list() {
    ///     println!("Registered: {}", name);
    /// }
    /// ```
    pub fn list(&self) -> Vec<String> {
        self.registry.list_types()
    }

    // === LFO (Top-level - commonly used) ===

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
    /// - `mode` (LfoMode) - Free running or beat synced
    ///
    /// # Example
    /// ```ignore
    /// dsp.lfo("bass_lfo", LfoShape::Sine, 0.5)?;
    /// let lfo = engine.instance("bass_lfo", &params! { "depth" => 0.8 });
    /// ```
    pub fn lfo(&self, name: impl Into<String>, shape: LfoShape, frequency: f32) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let freq = get_param_or(params, "frequency", frequency);
            let mut lfo = LfoNode::new(shape, freq);
            AudioUnit::set_sample_rate(&mut lfo, sample_rate);

            // Optional parameters
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

    // === Envelope Follower (Top-level - commonly used) ===

    /// Register a peak envelope follower node
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    ///
    /// # Instance Parameters
    /// - `gain` (f32) - Output gain multiplier
    ///
    /// # Example
    /// ```ignore
    /// dsp.envelope("env", 0.001, 0.1)?;
    /// let env = engine.instance("env", &params! { "gain" => 2.0 });
    /// ```
    pub fn envelope(&self, name: impl Into<String>, attack_sec: f32, release_sec: f32) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let mut env = EnvelopeFollowerNode::new(attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut env, sample_rate);

            if let Some(gain) = params.get("gain").and_then(|v| v.as_f32()) {
                env.set_gain(gain);
            }

            Ok(Box::new(env) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register an RMS envelope follower node
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    /// * `window_ms` - RMS window size in milliseconds
    ///
    /// # Instance Parameters
    /// - `gain` (f32) - Output gain multiplier
    ///
    /// # Example
    /// ```ignore
    /// dsp.rms_envelope("rms_env", 0.001, 0.1, 10.0)?;
    /// ```
    pub fn rms_envelope(
        &self,
        name: impl Into<String>,
        attack_sec: f32,
        release_sec: f32,
        window_ms: f32,
    ) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let mut env = EnvelopeFollowerNode::new_rms(attack_sec, release_sec, window_ms);
            AudioUnit::set_sample_rate(&mut env, sample_rate);

            if let Some(gain) = params.get("gain").and_then(|v| v.as_f32()) {
                env.set_gain(gain);
            }

            Ok(Box::new(env) as Box<dyn AudioUnit>)
        });

        self
    }

    // === Grouped Handles ===

    /// Get sidechain dynamics handle (compressor, gate, limiter, expander)
    ///
    /// # Example
    /// ```ignore
    /// dsp.sidechain().compressor("comp", -20.0, 4.0, 0.001, 0.05)?;
    /// ```
    pub fn sidechain(&self) -> SidechainHandle<'a> {
        SidechainHandle::new(self.registry, self.sample_rate)
    }

    /// Get spatial audio handle (VBAP, binaural)
    ///
    /// # Example
    /// ```ignore
    /// dsp.spatial().vbap("panner", ChannelLayout::Stereo)?;
    /// dsp.spatial().binaural("binaural")?;
    /// ```
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
    /// Create a new sidechain handle
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    /// Register a sidechain compressor (mono)
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `threshold_db` - Compression threshold in dB
    /// * `ratio` - Compression ratio (e.g., 4.0 for 4:1)
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    ///
    /// # Input Channels
    /// - Channel 0: Audio input
    /// - Channel 1: Sidechain input
    ///
    /// # Example
    /// ```ignore
    /// dsp.sidechain().compressor("comp", -20.0, 4.0, 0.001, 0.05)?;
    /// let comp = engine.instance("comp", &params! {});
    /// ```
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
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `threshold_db` - Gate threshold in dB
    /// * `ratio` - Gate ratio (higher = more aggressive)
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    ///
    /// # Input Channels
    /// - Channel 0: Audio input
    /// - Channel 1: Sidechain input
    ///
    /// # Example
    /// ```ignore
    /// dsp.sidechain().gate("gate", -40.0, 10.0, 0.001, 0.1)?;
    /// ```
    pub fn gate(
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
            let mut gate = SidechainGate::new(threshold_db, ratio, attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut gate, sample_rate);
            Ok(Box::new(gate) as Box<dyn AudioUnit>)
        });

        self
    }

    /// Register a stereo sidechain compressor
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `threshold_db` - Compression threshold in dB
    /// * `ratio` - Compression ratio (e.g., 4.0 for 4:1)
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    ///
    /// # Input Channels
    /// - Channels 0-1: Stereo audio input (L/R)
    /// - Channels 2-3: Stereo sidechain input (L/R)
    ///
    /// # Example
    /// ```ignore
    /// dsp.sidechain().stereo_compressor("stereo_comp", -20.0, 4.0, 0.001, 0.05)?;
    /// ```
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
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `threshold_db` - Gate threshold in dB
    /// * `ratio` - Gate ratio (higher = more aggressive)
    /// * `attack_sec` - Attack time in seconds
    /// * `release_sec` - Release time in seconds
    ///
    /// # Input Channels
    /// - Channels 0-1: Stereo audio input (L/R)
    /// - Channels 2-3: Stereo sidechain input (L/R)
    ///
    /// # Example
    /// ```ignore
    /// dsp.sidechain().stereo_gate("stereo_gate", -40.0, 10.0, 0.001, 0.1)?;
    /// ```
    pub fn stereo_gate(
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
            let mut gate = StereoSidechainGate::new(threshold_db, ratio, attack_sec, release_sec);
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
    /// Create a new spatial handle
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    /// Register a VBAP (Vector Base Amplitude Panning) spatial panner
    ///
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    /// * `layout` - Speaker layout (Stereo, Quad, Surround_5_1, Surround_7_1, Atmos_7_1_4)
    ///
    /// # Instance Parameters
    /// - `azimuth` (f32) - Horizontal angle in degrees (-180 to 180, 0 = front)
    /// - `elevation` (f32) - Vertical angle in degrees (-90 to 90, 0 = ear level)
    /// - `spread` (f32) - Spread factor (0.0 = point source, 1.0 = diffuse)
    ///
    /// # Example
    /// ```ignore
    /// use tutti::dsp_nodes::ChannelLayout;
    ///
    /// dsp.spatial().vbap("panner", ChannelLayout::Stereo)?;
    /// let panner = engine.instance("panner", &params! {
    ///     "azimuth" => 45.0,
    ///     "elevation" => 0.0,
    /// });
    /// ```
    pub fn vbap(&self, name: impl Into<String>, layout: ChannelLayout) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            // Create panner based on layout
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
                })?, // Default to stereo
            };

            AudioUnit::set_sample_rate(&mut panner, sample_rate);

            // Apply instance parameters
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
    /// # Arguments
    /// * `name` - Unique identifier for this node type
    ///
    /// # Instance Parameters
    /// - `azimuth` (f32) - Horizontal angle in degrees (-180 to 180, 0 = front)
    /// - `elevation` (f32) - Vertical angle in degrees (-90 to 90, 0 = ear level)
    ///
    /// # Example
    /// ```ignore
    /// dsp.spatial().binaural("binaural")?;
    /// let panner = engine.instance("binaural", &params! {
    ///     "azimuth" => 90.0,  // Hard left
    ///     "elevation" => 0.0,
    /// });
    /// ```
    pub fn binaural(&self, name: impl Into<String>) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let panner = BinauralPannerNode::new(sample_rate as f32);

            // Apply instance parameters
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
