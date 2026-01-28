use crate::spatial::types::ChannelLayout;
use crate::Result;
use std::sync::Arc;
use tutti_core::AtomicFloat;
use vbap::{SpeakerConfig, VBAPanner};

/// Default smoothing time for position changes (50ms for smooth automation)
const DEFAULT_POSITION_SMOOTH_TIME: f32 = 0.05;

/// Simple exponential smoothing filter for real-time parameter smoothing
/// This replaces FunDSP's Follow filter with a simpler implementation
struct ExponentialSmoother {
    /// Current smoothed value
    value: f32,
    /// Smoothing coefficient (0 = no smoothing, 1 = instant)
    coeff: f32,
}

impl ExponentialSmoother {
    /// Create a new smoother with smoothing time in seconds
    /// At 48kHz, 0.05s = 2400 samples. We want to reach ~99% in that time.
    /// tau = -1 / ln(1 - target_level), for 99% convergence: tau ≈ 4.6
    /// coeff = 1 - exp(-1 / (time * sample_rate))
    fn new(smooth_time: f32, sample_rate: f32) -> Self {
        let coeff = 1.0 - (-1.0 / (smooth_time * sample_rate)).exp();
        Self {
            value: 0.0,
            coeff: coeff.clamp(0.0, 1.0),
        }
    }

    /// Process one sample (returns smoothed value)
    fn process(&mut self, target: f32) -> f32 {
        self.value += self.coeff * (target - self.value);
        self.value
    }

    /// Reset to a specific value
    #[allow(dead_code)]
    fn reset(&mut self, value: f32) {
        self.value = value;
    }
}

/// Spatial audio panner using VBAP
///
/// Wraps the vbap crate with DAW-friendly API and channel layout handling.
pub struct SpatialPanner {
    panner: VBAPanner,
    layout: ChannelLayout,
    /// Target azimuth position in degrees (-180 to 180)
    azimuth_target: Arc<AtomicFloat>,
    /// Target elevation position in degrees (-90 to 90)
    elevation_target: Arc<AtomicFloat>,
    /// Azimuth smoother
    azimuth_smoother: ExponentialSmoother,
    /// Elevation smoother
    elevation_smoother: ExponentialSmoother,
    /// Spread factor (0.0 = point source, 1.0 = diffuse)
    spread: f32,
    /// Sample rate for smoothing (stored for potential runtime changes)
    #[allow(dead_code)]
    sample_rate: f32,
}

impl SpatialPanner {
    /// Helper to create panner with default smoothing
    fn new_with_layout(panner: VBAPanner, layout: ChannelLayout) -> Self {
        let sample_rate = 48000.0; // Default sample rate, can be updated later
        Self {
            panner,
            layout,
            azimuth_target: Arc::new(AtomicFloat::new(0.0)),
            elevation_target: Arc::new(AtomicFloat::new(0.0)),
            azimuth_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            elevation_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            spread: 0.0,
            sample_rate,
        }
    }

    /// Create a stereo panner
    pub fn stereo() -> Result<Self> {
        let panner = VBAPanner::builder().stereo().build()?;
        Ok(Self::new_with_layout(panner, ChannelLayout::stereo()))
    }

    /// Create a quad (4.0) panner
    pub fn quad() -> Result<Self> {
        let panner = VBAPanner::builder().quad().build()?;
        Ok(Self::new_with_layout(
            panner,
            ChannelLayout {
                left: 0,
                right: 1,
                center: None,
                lfe: None,
                surround_left: Some(2),
                surround_right: Some(3),
                rear_left: None,
                rear_right: None,
                height_front_left: None,
                height_front_right: None,
                height_rear_left: None,
                height_rear_right: None,
            },
        ))
    }

    /// Create a 5.1 surround panner
    pub fn surround_5_1() -> Result<Self> {
        let panner = VBAPanner::builder().surround_5_1().build()?;
        Ok(Self::new_with_layout(panner, ChannelLayout::surround_5_1()))
    }

    /// Create a 7.1 surround panner
    pub fn surround_7_1() -> Result<Self> {
        let panner = VBAPanner::builder().surround_7_1().build()?;
        Ok(Self::new_with_layout(panner, ChannelLayout::surround_7_1()))
    }

    /// Create a Dolby Atmos 7.1.4 panner
    pub fn atmos_7_1_4() -> Result<Self> {
        let panner = VBAPanner::builder().atmos_7_1_4().build()?;
        Ok(Self::new_with_layout(panner, ChannelLayout::atmos_7_1_4()))
    }

    /// Create a panner with custom speaker configuration
    pub fn from_config(config: SpeakerConfig) -> Result<Self> {
        let num_speakers = config.num_speakers();
        let panner = VBAPanner::new(config);

        let layout = match num_speakers {
            2 => ChannelLayout::stereo(),
            6 => ChannelLayout::surround_5_1(),
            8 => ChannelLayout::surround_7_1(),
            12 => ChannelLayout::atmos_7_1_4(),
            _ => ChannelLayout {
                left: 0,
                right: std::cmp::min(1, num_speakers - 1),
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
            },
        };

        Ok(Self::new_with_layout(panner, layout))
    }

    /// Create a panner with custom speaker positions
    ///
    /// Each position is (azimuth, elevation) in degrees.
    pub fn custom(positions: &[(f64, f64)]) -> Result<Self> {
        let mut builder = VBAPanner::builder();
        for &(az, el) in positions {
            builder = builder.add_speaker(az, el);
        }
        let panner = builder.build()?;

        let num_speakers = panner.num_speakers();
        let layout = match num_speakers {
            2 => ChannelLayout::stereo(),
            6 => ChannelLayout::surround_5_1(),
            8 => ChannelLayout::surround_7_1(),
            12 => ChannelLayout::atmos_7_1_4(),
            _ => ChannelLayout {
                left: 0,
                right: std::cmp::min(1, num_speakers.saturating_sub(1)),
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
            },
        };

        Ok(Self::new_with_layout(panner, layout))
    }

    /// Get the channel layout
    pub fn layout(&self) -> &ChannelLayout {
        &self.layout
    }

    /// Get number of output channels
    pub fn num_channels(&self) -> usize {
        self.panner.num_speakers()
    }

    /// Set position in degrees (smoothed over 50ms)
    ///
    /// Uses VBAP angle convention:
    /// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left, -90 = right)
    /// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
    pub fn set_position(&mut self, azimuth: f32, elevation: f32) {
        self.azimuth_target.set(azimuth.clamp(-180.0, 180.0));
        self.elevation_target.set(elevation.clamp(-90.0, 90.0));
    }

    /// Get current azimuth (target value)
    pub fn azimuth(&self) -> f32 {
        self.azimuth_target.get()
    }

    /// Get current elevation (target value)
    pub fn elevation(&self) -> f32 {
        self.elevation_target.get()
    }

    /// Set spread factor (0.0 = point source, 1.0 = diffuse)
    pub fn set_spread(&mut self, spread: f32) {
        self.spread = spread.clamp(0.0, 1.0);
    }

    /// Get current spread
    pub fn spread(&self) -> f32 {
        self.spread
    }

    /// Compute speaker gains for current position
    ///
    /// Returns a vector of gains, one per speaker channel.
    /// Updates smoothed position values on each call.
    pub fn compute_gains(&mut self) -> Vec<f32> {
        let target_azimuth = self.azimuth_target.get();
        let target_elevation = self.elevation_target.get();

        let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
        let smoothed_elevation = self.elevation_smoother.process(target_elevation);

        let gains_f64 = self
            .panner
            .compute_gains(smoothed_azimuth as f64, smoothed_elevation as f64);
        let mut gains: Vec<f32> = gains_f64.iter().map(|&g| g as f32).collect();

        // Apply spread: blend toward equal power across all speakers
        if self.spread > 0.0 {
            let equal_gain = 1.0 / (self.num_channels() as f32).sqrt();
            for gain in &mut gains {
                *gain = *gain * (1.0 - self.spread) + equal_gain * self.spread;
            }
            // Renormalize to maintain energy
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

    /// Compute gains for a specific position (without changing stored position)
    pub fn compute_gains_at(&self, azimuth: f32, elevation: f32) -> Vec<f32> {
        let gains_f64 = self.panner.compute_gains(azimuth as f64, elevation as f64);
        gains_f64.iter().map(|&g| g as f32).collect()
    }

    /// Process a mono sample into multichannel output
    pub fn process_mono(&mut self, sample: f32) -> Vec<f32> {
        self.compute_gains().iter().map(|g| sample * g).collect()
    }

    /// Process mono into pre-allocated output buffer
    pub fn process_mono_into(&mut self, sample: f32, output: &mut [f32]) {
        let gains = self.compute_gains();
        for (out, gain) in output.iter_mut().zip(gains.iter()) {
            *out = sample * gain;
        }
    }

    /// Process a block of mono samples into multichannel output
    ///
    /// Output is interleaved: [ch0_s0, ch1_s0, ..., chN_s0, ch0_s1, ch1_s1, ...]
    pub fn process_mono_block(&mut self, input: &[f32]) -> Vec<f32> {
        let gains = self.compute_gains();
        let num_channels = gains.len();
        let mut output = Vec::with_capacity(input.len() * num_channels);

        for &sample in input {
            for &gain in &gains {
                output.push(sample * gain);
            }
        }

        output
    }

    /// Process stereo into multichannel with width preservation
    ///
    /// Uses stereo width parameter to blend between mono (width=0.0) and full stereo (width=1.0).
    /// Creates phantom image by panning L/R to slightly different positions.
    ///
    /// # Arguments
    /// * `left` - Left channel sample
    /// * `right` - Right channel sample
    /// * `width` - Stereo width (0.0 = mono, 1.0 = full stereo, >1.0 = exaggerated)
    /// * `output` - Multichannel output buffer
    pub fn process_stereo_into(&mut self, left: f32, right: f32, width: f32, output: &mut [f32]) {
        let width = width.max(0.0); // Clamp to non-negative

        if width < 0.001 {
            // Effectively mono - just downmix
            let mono = (left + right) * 0.5;
            self.process_mono_into(mono, output);
        } else {
            let target_azimuth = self.azimuth_target.get();
            let target_elevation = self.elevation_target.get();

            let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
            let smoothed_elevation = self.elevation_smoother.process(target_elevation);

            // Stereo upmixing: pan L/R to offset positions
            // Width of ±15° gives good stereo imaging in surround
            let angle_offset = 15.0 * width;

            // Compute gains for left source (slightly left of center position)
            let az_left = smoothed_azimuth + angle_offset;
            let gains_left = self
                .panner
                .compute_gains(az_left as f64, smoothed_elevation as f64);

            // Compute gains for right source (slightly right of center position)
            let az_right = smoothed_azimuth - angle_offset;
            let gains_right = self
                .panner
                .compute_gains(az_right as f64, smoothed_elevation as f64);

            // Mix both into output
            for (i, out) in output.iter_mut().enumerate() {
                let gain_l = gains_left.get(i).copied().unwrap_or(0.0) as f32;
                let gain_r = gains_right.get(i).copied().unwrap_or(0.0) as f32;
                *out = left * gain_l + right * gain_r;
            }
        }
    }

    /// Process a block of mono samples with position automation
    ///
    /// `positions` should contain (azimuth, elevation) pairs for each sample.
    /// Output is interleaved multichannel.
    pub fn process_mono_block_automated(
        &self,
        input: &[f32],
        positions: &[(f32, f32)],
    ) -> Vec<f32> {
        let num_channels = self.num_channels();
        let mut output = Vec::with_capacity(input.len() * num_channels);

        for (i, &sample) in input.iter().enumerate() {
            let (az, el) = positions
                .get(i)
                .copied()
                .unwrap_or((self.azimuth(), self.elevation()));
            let gains_f64 = self.panner.compute_gains(az as f64, el as f64);
            for gain in gains_f64 {
                output.push(sample * gain as f32);
            }
        }

        output
    }
}
