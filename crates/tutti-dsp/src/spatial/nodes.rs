//! Spatial AudioUnit nodes for FunDSP Net integration
//!
//! Provides AudioUnit wrappers for spatial panners:
//! - `SpatialPannerNode`: VBAP-based multichannel panner (5.1, 7.1, Atmos)
//! - `BinauralPannerNode`: ITD/ILD headphone 3D audio
//!
//! ## Example
//!
//! ```rust,ignore
//! use tutti::net::TuttiNet;
//! use tutti::spatial::{SpatialPannerNode, BinauralPannerNode};
//!
//! // Create a 5.1 surround panner node
//! let panner = SpatialPannerNode::surround_5_1()?;
//! panner.set_position(45.0, 0.0); // Front-left
//!
//! // Add to Net (mono input → 6 channel output)
//! let node_id = net.add(Box::new(panner));
//! net.connect(synth_id, 0, node_id, 0);
//! net.commit();
//!
//! // Or use binaural for headphones (mono → stereo)
//! let binaural = BinauralPannerNode::new(48000.0);
//! binaural.set_position(90.0, 0.0); // Hard left
//! ```

use crate::Result;
use core::sync::atomic::{AtomicU32, Ordering};
use tutti_core::Arc;
use tutti_core::AudioUnit;
use tutti_core::{BufferMut, BufferRef, SignalFrame};

use super::binaural_panner::BinauralPanner;
use super::vbap_panner::SpatialPanner;

/// VBAP-based spatial panner as an AudioUnit node
///
/// Takes mono input and outputs to multiple speakers using Vector Base
/// Amplitude Panning (VBAP). Supports various speaker configurations:
/// - Stereo (2 channels)
/// - Quad (4 channels)
/// - 5.1 Surround (6 channels)
/// - 7.1 Surround (8 channels)
/// - Dolby Atmos 7.1.4 (12 channels)
///
/// ## Position Control
///
/// Position is controlled via atomic floats for real-time automation:
/// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left)
/// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
///
/// ## Example
///
/// ```rust,ignore
/// let panner = SpatialPannerNode::surround_5_1()?;
/// panner.set_position(45.0, 0.0); // Front-left at ear level
///
/// let node_id = net.add(Box::new(panner));
/// net.connect(synth_id, 0, node_id, 0); // Connect mono synth
/// net.pipe_output(node_id); // Route to 5.1 output
/// ```
pub struct SpatialPannerNode {
    panner: SpatialPanner,
    num_outputs: usize,
    azimuth_atomic: Arc<AtomicU32>,
    elevation_atomic: Arc<AtomicU32>,
    spread_atomic: Arc<AtomicU32>,
    width_atomic: Arc<AtomicU32>,
    sample_rate: f32,
    scratch_output: Vec<f32>,
}

impl Clone for SpatialPannerNode {
    fn clone(&self) -> Self {
        // Re-create the panner with current position so the backend
        // starts from the correct state.
        let mut new_panner = match self.num_outputs {
            2 => SpatialPanner::stereo().expect("stereo preset"),
            4 => SpatialPanner::quad().expect("quad preset"),
            6 => SpatialPanner::surround_5_1().expect("5.1 preset"),
            8 => SpatialPanner::surround_7_1().expect("7.1 preset"),
            12 => SpatialPanner::atmos_7_1_4().expect("Atmos preset"),
            _ => SpatialPanner::stereo().expect("stereo fallback"),
        };

        let azimuth = f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed));
        let elevation = f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed));
        let spread = f32::from_bits(self.spread_atomic.load(Ordering::Relaxed));
        new_panner.set_position(azimuth, elevation);
        new_panner.set_spread(spread);

        Self {
            panner: new_panner,
            num_outputs: self.num_outputs,
            // Share the same Arc so frontend and backend see the same atomics
            azimuth_atomic: Arc::clone(&self.azimuth_atomic),
            elevation_atomic: Arc::clone(&self.elevation_atomic),
            spread_atomic: Arc::clone(&self.spread_atomic),
            width_atomic: Arc::clone(&self.width_atomic),
            sample_rate: self.sample_rate,
            scratch_output: vec![0.0; self.num_outputs],
        }
    }
}

impl SpatialPannerNode {
    /// Create a stereo spatial panner (2 channels)
    pub fn stereo() -> Result<Self> {
        let panner = SpatialPanner::stereo()?;
        Ok(Self::from_panner(panner, 2))
    }

    /// Create a quad spatial panner (4 channels)
    pub fn quad() -> Result<Self> {
        let panner = SpatialPanner::quad()?;
        Ok(Self::from_panner(panner, 4))
    }

    /// Create a 5.1 surround panner (6 channels)
    pub fn surround_5_1() -> Result<Self> {
        let panner = SpatialPanner::surround_5_1()?;
        Ok(Self::from_panner(panner, 6))
    }

    /// Create a 7.1 surround panner (8 channels)
    pub fn surround_7_1() -> Result<Self> {
        let panner = SpatialPanner::surround_7_1()?;
        Ok(Self::from_panner(panner, 8))
    }

    /// Create a Dolby Atmos 7.1.4 panner (12 channels)
    pub fn atmos_7_1_4() -> Result<Self> {
        let panner = SpatialPanner::atmos_7_1_4()?;
        Ok(Self::from_panner(panner, 12))
    }

    /// Create from an existing SpatialPanner
    fn from_panner(panner: SpatialPanner, num_outputs: usize) -> Self {
        Self {
            panner,
            num_outputs,
            azimuth_atomic: Arc::new(AtomicU32::new(0.0_f32.to_bits())),
            elevation_atomic: Arc::new(AtomicU32::new(0.0_f32.to_bits())),
            spread_atomic: Arc::new(AtomicU32::new(0.0_f32.to_bits())),
            width_atomic: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            sample_rate: 48000.0,
            scratch_output: vec![0.0; num_outputs],
        }
    }

    /// Set position in degrees (thread-safe, lock-free)
    ///
    /// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left, -90 = right)
    /// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
    pub fn set_position(&self, azimuth: f32, elevation: f32) {
        self.azimuth_atomic
            .store(azimuth.to_bits(), Ordering::Relaxed);
        self.elevation_atomic
            .store(elevation.to_bits(), Ordering::Relaxed);
    }

    /// Get current azimuth
    pub fn azimuth(&self) -> f32 {
        f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed))
    }

    /// Get current elevation
    pub fn elevation(&self) -> f32 {
        f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed))
    }

    /// Set spread factor (0.0 = point source, 1.0 = diffuse)
    pub fn set_spread(&self, spread: f32) {
        self.spread_atomic
            .store(spread.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Get current spread
    pub fn spread(&self) -> f32 {
        f32::from_bits(self.spread_atomic.load(Ordering::Relaxed))
    }

    /// Set stereo width for stereo input mode (0.0 = mono, 1.0 = full stereo)
    pub fn set_width(&self, width: f32) {
        self.width_atomic
            .store(width.max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// Get current stereo width
    pub fn width(&self) -> f32 {
        f32::from_bits(self.width_atomic.load(Ordering::Relaxed))
    }

    /// Get the number of output channels
    pub fn num_channels(&self) -> usize {
        self.num_outputs
    }

    /// Update internal panner state from atomics
    #[inline]
    fn sync_position(&mut self) {
        let azimuth = f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed));
        let elevation = f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed));
        let spread = f32::from_bits(self.spread_atomic.load(Ordering::Relaxed));
        self.panner.set_position(azimuth, elevation);
        self.panner.set_spread(spread);
    }
}

impl AudioUnit for SpatialPannerNode {
    fn inputs(&self) -> usize {
        2
    }

    fn outputs(&self) -> usize {
        self.num_outputs
    }

    fn reset(&mut self) {
        self.azimuth_atomic
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.elevation_atomic
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.spread_atomic
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.width_atomic
            .store(1.0_f32.to_bits(), Ordering::Relaxed);
        self.panner.set_position(0.0, 0.0);
        self.panner.set_spread(0.0);
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        self.sync_position();

        let width = f32::from_bits(self.width_atomic.load(Ordering::Relaxed));

        let left = input.first().copied().unwrap_or(0.0);
        let right = input.get(1).copied().unwrap_or(left);
        self.panner.process_stereo_into(left, right, width, output);
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.sync_position();

        let width = f32::from_bits(self.width_atomic.load(Ordering::Relaxed));
        let num_outputs = self.num_outputs;

        if self.scratch_output.len() < num_outputs {
            self.scratch_output.resize(num_outputs, 0.0);
        }

        for i in 0..size {
            let left = input.at_f32(0, i);
            let right = if input.channels() > 1 {
                input.at_f32(1, i)
            } else {
                left
            };

            self.panner
                .process_stereo_into(left, right, width, &mut self.scratch_output);

            for (ch, &sample) in self.scratch_output.iter().enumerate() {
                if ch < num_outputs {
                    output.set_f32(ch, i, sample);
                }
            }
        }
    }

    fn get_id(&self) -> u64 {
        0x5041_4E00 | (self.num_outputs as u64)
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(self.num_outputs);
        for i in 0..self.num_outputs {
            output.set(i, input.at(0));
        }
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

/// Binaural panner as an AudioUnit node
///
/// Takes mono input and outputs binaural stereo for headphone listening.
/// Uses simple ITD (Interaural Time Difference) and ILD (Interaural Level
/// Difference) model for 3D audio spatialization.
///
/// ## Position Control
///
/// Position is controlled via atomic floats for real-time automation:
/// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left)
/// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
///
/// ## Example
///
/// ```rust,ignore
/// let panner = BinauralPannerNode::new(48000.0);
/// panner.set_position(90.0, 0.0); // Hard left
///
/// let node_id = net.add(Box::new(panner));
/// net.connect(synth_id, 0, node_id, 0); // Connect mono synth
/// net.pipe_output(node_id); // Route to stereo output
/// ```
pub struct BinauralPannerNode {
    panner: BinauralPanner,
    azimuth_atomic: Arc<AtomicU32>,
    elevation_atomic: Arc<AtomicU32>,
    width_atomic: Arc<AtomicU32>,
    sample_rate: f32,
}

impl Clone for BinauralPannerNode {
    fn clone(&self) -> Self {
        let mut new_panner = BinauralPanner::new(self.sample_rate);
        let azimuth = f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed));
        let elevation = f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed));
        new_panner.set_position(azimuth, elevation);

        Self {
            panner: new_panner,
            azimuth_atomic: Arc::clone(&self.azimuth_atomic),
            elevation_atomic: Arc::clone(&self.elevation_atomic),
            width_atomic: Arc::clone(&self.width_atomic),
            sample_rate: self.sample_rate,
        }
    }
}

impl BinauralPannerNode {
    /// Create a new binaural panner node
    pub fn new(sample_rate: f32) -> Self {
        Self {
            panner: BinauralPanner::new(sample_rate),
            azimuth_atomic: Arc::new(AtomicU32::new(0.0_f32.to_bits())),
            elevation_atomic: Arc::new(AtomicU32::new(0.0_f32.to_bits())),
            width_atomic: Arc::new(AtomicU32::new(1.0_f32.to_bits())),
            sample_rate,
        }
    }

    /// Set position in degrees (thread-safe, lock-free)
    ///
    /// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left, -90 = right)
    /// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
    pub fn set_position(&self, azimuth: f32, elevation: f32) {
        self.azimuth_atomic
            .store(azimuth.to_bits(), Ordering::Relaxed);
        self.elevation_atomic
            .store(elevation.to_bits(), Ordering::Relaxed);
    }

    /// Get current azimuth
    pub fn azimuth(&self) -> f32 {
        f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed))
    }

    /// Get current elevation
    pub fn elevation(&self) -> f32 {
        f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed))
    }

    /// Set stereo width for stereo input mode (0.0 = mono, 1.0 = full stereo)
    pub fn set_width(&self, width: f32) {
        self.width_atomic
            .store(width.clamp(0.0, 2.0).to_bits(), Ordering::Relaxed);
    }

    /// Get current stereo width
    pub fn width(&self) -> f32 {
        f32::from_bits(self.width_atomic.load(Ordering::Relaxed))
    }

    /// Update internal panner state from atomics
    #[inline]
    fn sync_position(&mut self) {
        let azimuth = f32::from_bits(self.azimuth_atomic.load(Ordering::Relaxed));
        let elevation = f32::from_bits(self.elevation_atomic.load(Ordering::Relaxed));
        self.panner.set_position(azimuth, elevation);
    }
}

impl AudioUnit for BinauralPannerNode {
    fn inputs(&self) -> usize {
        2
    }

    fn outputs(&self) -> usize {
        2
    }

    fn reset(&mut self) {
        self.azimuth_atomic
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.elevation_atomic
            .store(0.0_f32.to_bits(), Ordering::Relaxed);
        self.width_atomic
            .store(1.0_f32.to_bits(), Ordering::Relaxed);
        self.panner = BinauralPanner::new(self.sample_rate);
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
        self.panner = BinauralPanner::new(self.sample_rate);
    }

    fn tick(&mut self, input: &[f32], output: &mut [f32]) {
        self.sync_position();

        let width = f32::from_bits(self.width_atomic.load(Ordering::Relaxed));

        let left = input.first().copied().unwrap_or(0.0);
        let right = input.get(1).copied().unwrap_or(left);

        let (out_left, out_right) = self.panner.process_stereo(left, right, width);

        if output.len() >= 2 {
            output[0] = out_left;
            output[1] = out_right;
        }
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        self.sync_position();

        let width = f32::from_bits(self.width_atomic.load(Ordering::Relaxed));

        for i in 0..size {
            let left = input.at_f32(0, i);
            let right = if input.channels() > 1 {
                input.at_f32(1, i)
            } else {
                left
            };

            let (out_left, out_right) = self.panner.process_stereo(left, right, width);
            output.set_f32(0, i, out_left);
            output.set_f32(1, i, out_right);
        }
    }

    fn get_id(&self) -> u64 {
        0x4249_4E00
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
        self
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        let mut output = SignalFrame::new(2);
        output.set(0, input.at(0));
        output.set(1, input.at(0));
        output
    }

    fn footprint(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatial_panner_tick() {
        let mut panner = SpatialPannerNode::stereo().unwrap();
        panner.set_position(0.0, 0.0);

        let input = [1.0f32, 1.0f32];
        let mut output = [0.0f32; 2];

        panner.tick(&input, &mut output);

        assert!(output[0] > 0.0);
        assert!(output[1] > 0.0);
    }

    #[test]
    fn test_binaural_panner_tick() {
        let mut panner = BinauralPannerNode::new(48000.0);
        panner.set_position(90.0, 0.0);

        let input = [1.0f32, 1.0f32];
        let mut output = [0.0f32; 2];
        for _ in 0..100 {
            panner.tick(&input, &mut output);
        }

        panner.tick(&input, &mut output);
        assert!(
            output[0] > output[1],
            "Left should be louder for left position"
        );
    }

    #[test]
    fn test_spatial_panner_clone() {
        let panner = SpatialPannerNode::surround_5_1().unwrap();
        panner.set_position(45.0, 15.0);
        panner.set_spread(0.3);

        let cloned = panner.clone();

        assert_eq!(cloned.num_channels(), panner.num_channels());
        assert!((cloned.azimuth() - panner.azimuth()).abs() < 0.001);
        assert!((cloned.elevation() - panner.elevation()).abs() < 0.001);
        assert!((cloned.spread() - panner.spread()).abs() < 0.001);
    }

    #[test]
    fn test_binaural_panner_clone() {
        let panner = BinauralPannerNode::new(44100.0);
        panner.set_position(-45.0, 10.0);
        panner.set_width(0.5);

        let cloned = panner.clone();

        assert!((cloned.azimuth() - panner.azimuth()).abs() < 0.001);
        assert!((cloned.elevation() - panner.elevation()).abs() < 0.001);
        assert!((cloned.width() - panner.width()).abs() < 0.001);
    }

    #[test]
    fn test_spatial_clone_shares_atomics() {
        let panner = SpatialPannerNode::stereo().unwrap();
        let cloned = panner.clone();

        // Setting position on original should be visible from clone
        panner.set_position(90.0, 45.0);
        assert!((cloned.azimuth() - 90.0).abs() < 0.001);
        assert!((cloned.elevation() - 45.0).abs() < 0.001);

        // And vice versa
        cloned.set_position(-60.0, 10.0);
        assert!((panner.azimuth() - (-60.0)).abs() < 0.001);
        assert!((panner.elevation() - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_binaural_clone_shares_atomics() {
        let panner = BinauralPannerNode::new(48000.0);
        let cloned = panner.clone();

        panner.set_position(90.0, 45.0);
        assert!((cloned.azimuth() - 90.0).abs() < 0.001);
        assert!((cloned.elevation() - 45.0).abs() < 0.001);

        cloned.set_position(-60.0, 10.0);
        assert!((panner.azimuth() - (-60.0)).abs() < 0.001);
        assert!((panner.elevation() - 10.0).abs() < 0.001);
    }
}
