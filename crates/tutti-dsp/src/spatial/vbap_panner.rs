use crate::Result;
use tutti_core::Arc;
use tutti_core::AtomicFloat;
use vbap::VBAPanner;

use super::utils::{ExponentialSmoother, DEFAULT_POSITION_SMOOTH_TIME};

/// Spatial audio panner using VBAP
///
/// Wraps the vbap crate with DAW-friendly API and channel layout handling.
/// **Internal implementation detail** - users should use `SpatialPannerNode` instead.
pub(crate) struct SpatialPanner {
    panner: VBAPanner,
    azimuth_target: Arc<AtomicFloat>,
    elevation_target: Arc<AtomicFloat>,
    azimuth_smoother: ExponentialSmoother,
    elevation_smoother: ExponentialSmoother,
    spread: f32,
    #[allow(dead_code)]
    sample_rate: f32,
}

impl SpatialPanner {
    fn new_with_layout(panner: VBAPanner) -> Self {
        let sample_rate = 48000.0;
        Self {
            panner,
            azimuth_target: Arc::new(AtomicFloat::new(0.0)),
            elevation_target: Arc::new(AtomicFloat::new(0.0)),
            azimuth_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            elevation_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            spread: 0.0,
            sample_rate,
        }
    }

    /// Create a stereo panner
    pub(crate) fn stereo() -> Result<Self> {
        let panner = VBAPanner::builder().stereo().build()?;
        Ok(Self::new_with_layout(panner))
    }

    /// Create a quad (4.0) panner
    pub(crate) fn quad() -> Result<Self> {
        let panner = VBAPanner::builder().quad().build()?;
        Ok(Self::new_with_layout(panner))
    }

    /// Create a 5.1 surround panner
    pub(crate) fn surround_5_1() -> Result<Self> {
        let panner = VBAPanner::builder().surround_5_1().build()?;
        Ok(Self::new_with_layout(panner))
    }

    /// Create a 7.1 surround panner
    pub(crate) fn surround_7_1() -> Result<Self> {
        let panner = VBAPanner::builder().surround_7_1().build()?;
        Ok(Self::new_with_layout(panner))
    }

    /// Create a Dolby Atmos 7.1.4 panner
    pub(crate) fn atmos_7_1_4() -> Result<Self> {
        let panner = VBAPanner::builder().atmos_7_1_4().build()?;
        Ok(Self::new_with_layout(panner))
    }

    /// Get number of output channels
    pub(crate) fn num_channels(&self) -> usize {
        self.panner.num_speakers()
    }

    /// Set position in degrees (smoothed over 50ms)
    ///
    /// Uses VBAP angle convention:
    /// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left, -90 = right)
    /// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
    pub(crate) fn set_position(&mut self, azimuth: f32, elevation: f32) {
        self.azimuth_target.set(azimuth.clamp(-180.0, 180.0));
        self.elevation_target.set(elevation.clamp(-90.0, 90.0));
    }

    /// Set spread factor (0.0 = point source, 1.0 = diffuse)
    pub(crate) fn set_spread(&mut self, spread: f32) {
        self.spread = spread.clamp(0.0, 1.0);
    }

    /// Compute speaker gains for current position
    ///
    /// Returns a vector of gains, one per speaker channel.
    /// Updates smoothed position values on each call.
    pub(crate) fn compute_gains(&mut self) -> Vec<f32> {
        let target_azimuth = self.azimuth_target.get();
        let target_elevation = self.elevation_target.get();

        let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
        let smoothed_elevation = self.elevation_smoother.process(target_elevation);

        let gains_f64 = self
            .panner
            .compute_gains(smoothed_azimuth as f64, smoothed_elevation as f64);
        let mut gains: Vec<f32> = gains_f64.iter().map(|&g| g as f32).collect();

        if self.spread > 0.0 {
            let equal_gain = 1.0 / (self.num_channels() as f32).sqrt();
            for gain in &mut gains {
                *gain = *gain * (1.0 - self.spread) + equal_gain * self.spread;
            }
            let sum_sq: f32 = gains.iter().map(|g| g * g).sum();
            if sum_sq > 0.0 {
                let norm = 1.0 / sum_sq.sqrt();
                for gain in &mut gains {
                    *gain *= norm;
                }
            }
        }

        gains
    }

    /// Process mono into pre-allocated output buffer
    pub(crate) fn process_mono_into(&mut self, sample: f32, output: &mut [f32]) {
        let gains = self.compute_gains();
        for (out, gain) in output.iter_mut().zip(gains.iter()) {
            *out = sample * gain;
        }
    }

    /// Process stereo into multichannel with width preservation
    pub(crate) fn process_stereo_into(
        &mut self,
        left: f32,
        right: f32,
        width: f32,
        output: &mut [f32],
    ) {
        let width = width.max(0.0);

        if width < 0.001 {
            let mono = (left + right) * 0.5;
            self.process_mono_into(mono, output);
        } else {
            let target_azimuth = self.azimuth_target.get();
            let target_elevation = self.elevation_target.get();

            let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
            let smoothed_elevation = self.elevation_smoother.process(target_elevation);

            let angle_offset = 15.0 * width;

            let az_left = smoothed_azimuth + angle_offset;
            let gains_left = self
                .panner
                .compute_gains(az_left as f64, smoothed_elevation as f64);

            let az_right = smoothed_azimuth - angle_offset;
            let gains_right = self
                .panner
                .compute_gains(az_right as f64, smoothed_elevation as f64);

            for (i, out) in output.iter_mut().enumerate() {
                let gain_l = gains_left.get(i).copied().unwrap_or(0.0) as f32;
                let gain_r = gains_right.get(i).copied().unwrap_or(0.0) as f32;
                *out = left * gain_l + right * gain_r;
            }
        }
    }
}
