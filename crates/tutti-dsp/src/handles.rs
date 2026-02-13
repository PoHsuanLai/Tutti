//! Fluent API handles for registering DSP nodes in the engine's node registry.

#[cfg(feature = "spatial")]
use crate::{BinauralPannerNode, ChannelLayout, SpatialPannerNode};
use crate::{LfoNode, LfoShape};
#[cfg(feature = "dynamics")]
use crate::{SidechainCompressor, SidechainGate, StereoSidechainCompressor, StereoSidechainGate};
use tutti_core::AudioUnit;
use tutti_core::NodeRegistry;

pub struct DspHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

impl<'a> DspHandle<'a> {
    pub fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    pub fn remove(&self, name: &str) -> bool {
        self.registry.unregister(name)
    }

    pub fn has(&self, name: &str) -> bool {
        self.registry.has_type(name)
    }

    pub fn list(&self) -> Vec<String> {
        self.registry.list_types()
    }

    pub fn lfo(&self, name: impl Into<String>, shape: LfoShape, frequency: f32) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register_simple(&name, move |p| {
            let freq: f32 = p.get_or("frequency", frequency);
            let mut lfo = LfoNode::new(shape, freq);
            AudioUnit::set_sample_rate(&mut lfo, sample_rate);

            if let Some(depth) = p.try_get::<f32>("depth") {
                lfo.set_depth(depth);
            }
            if let Some(phase) = p.try_get::<f32>("phase_offset") {
                lfo.set_phase_offset(phase);
            }

            lfo
        });

        self
    }

    #[cfg(feature = "dynamics")]
    pub fn sidechain(&self) -> SidechainHandle<'a> {
        SidechainHandle::new(self.registry, self.sample_rate)
    }

    #[cfg(feature = "spatial")]
    pub fn spatial(&self) -> SpatialHandle<'a> {
        SpatialHandle::new(self.registry, self.sample_rate)
    }
}

#[cfg(feature = "dynamics")]
pub struct SidechainHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

#[cfg(feature = "dynamics")]
impl<'a> SidechainHandle<'a> {
    pub(crate) fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

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

        self.registry.register_static(&name, move || {
            let mut comp = SidechainCompressor::new(threshold_db, ratio, attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut comp, sample_rate);
            comp
        });

        self
    }

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

        self.registry.register_static(&name, move || {
            let mut gate = SidechainGate::new(threshold_db, attack_sec, hold_sec, release_sec);
            AudioUnit::set_sample_rate(&mut gate, sample_rate);
            gate
        });

        self
    }

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

        self.registry.register_static(&name, move || {
            let mut comp =
                StereoSidechainCompressor::new(threshold_db, ratio, attack_sec, release_sec);
            AudioUnit::set_sample_rate(&mut comp, sample_rate);
            comp
        });

        self
    }

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

        self.registry.register_static(&name, move || {
            let mut gate =
                StereoSidechainGate::new(threshold_db, attack_sec, hold_sec, release_sec);
            AudioUnit::set_sample_rate(&mut gate, sample_rate);
            gate
        });

        self
    }
}

#[cfg(feature = "spatial")]
pub struct SpatialHandle<'a> {
    registry: &'a NodeRegistry,
    sample_rate: f64,
}

#[cfg(feature = "spatial")]
impl<'a> SpatialHandle<'a> {
    pub(crate) fn new(registry: &'a NodeRegistry, sample_rate: f64) -> Self {
        Self {
            registry,
            sample_rate,
        }
    }

    pub fn vbap(&self, name: impl Into<String>, layout: ChannelLayout) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register(&name, move |params| {
            let p = tutti_core::Params::new(params);
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

            if let Some(azimuth) = p.try_get::<f32>("azimuth") {
                let elevation: f32 = p.get_or("elevation", 0.0);
                panner.set_position(azimuth, elevation);
            }
            if let Some(spread) = p.try_get::<f32>("spread") {
                panner.set_spread(spread);
            }

            Ok(Box::new(panner) as Box<dyn AudioUnit>)
        });

        self
    }

    pub fn binaural(&self, name: impl Into<String>) -> &Self {
        let name = name.into();
        let sample_rate = self.sample_rate;

        self.registry.register_simple(&name, move |p| {
            let panner = BinauralPannerNode::new(sample_rate as f32);

            if let Some(azimuth) = p.try_get::<f32>("azimuth") {
                let elevation: f32 = p.get_or("elevation", 0.0);
                panner.set_position(azimuth, elevation);
            }

            panner
        });

        self
    }
}
