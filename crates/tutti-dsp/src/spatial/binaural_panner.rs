use tutti_core::Arc;
use tutti_core::AtomicFloat;

use super::utils::{ExponentialSmoother, DEFAULT_POSITION_SMOOTH_TIME};

/// Binaural panner using simple ITD/ILD model
///
/// Provides basic 3D audio for headphones without requiring HRTF datasets.
/// Uses Interaural Time Difference (ITD) and Interaural Level Difference (ILD)
/// to create spatial cues for headphone listening.
///
/// For production use, consider integrating a full HRTF library like:
/// - OpenAL Soft's HRTF dataset
/// - MIT KEMAR HRTF
/// - SADIE HRTF database
///
/// **Internal implementation detail** - users should use `BinauralPannerNode` instead.
pub(crate) struct BinauralPanner {
    /// Target azimuth position in degrees (-180 to 180)
    azimuth_target: Arc<AtomicFloat>,
    /// Target elevation position in degrees (-90 to 90)
    elevation_target: Arc<AtomicFloat>,
    /// Azimuth smoother
    azimuth_smoother: ExponentialSmoother,
    /// Elevation smoother
    elevation_smoother: ExponentialSmoother,
    /// Sample rate for ITD calculation
    sample_rate: f32,
    /// Simple delay line for ITD (max ~1ms at 48kHz = 48 samples)
    delay_buffer_left: Vec<f32>,
    delay_buffer_right: Vec<f32>,
    delay_write_pos: usize,
}

impl BinauralPanner {
    /// Create a new binaural panner
    ///
    /// `sample_rate` is needed for ITD (Interaural Time Difference) calculation
    pub(crate) fn new(sample_rate: f32) -> Self {
        const MAX_ITD_SAMPLES: usize = 64; // ~1.3ms at 48kHz (enough for max ITD)

        Self {
            azimuth_target: Arc::new(AtomicFloat::new(0.0)),
            elevation_target: Arc::new(AtomicFloat::new(0.0)),
            azimuth_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            elevation_smoother: ExponentialSmoother::new(DEFAULT_POSITION_SMOOTH_TIME, sample_rate),
            sample_rate,
            delay_buffer_left: vec![0.0; MAX_ITD_SAMPLES],
            delay_buffer_right: vec![0.0; MAX_ITD_SAMPLES],
            delay_write_pos: 0,
        }
    }

    /// Set position in degrees (smoothed over 50ms)
    ///
    /// - `azimuth`: Horizontal angle (-180 to 180, 0 = front, 90 = left, -90 = right)
    /// - `elevation`: Vertical angle (-90 to 90, 0 = ear level, positive = up)
    pub(crate) fn set_position(&mut self, azimuth: f32, elevation: f32) {
        self.azimuth_target.set(azimuth.clamp(-180.0, 180.0));
        self.elevation_target.set(elevation.clamp(-90.0, 90.0));
    }

    /// Process mono input to binaural stereo output
    ///
    /// Uses simple ITD/ILD model:
    /// - ITD (Interaural Time Difference): Sound reaches far ear later
    /// - ILD (Interaural Level Difference): Sound is quieter at far ear
    ///
    /// Returns (left, right) stereo samples
    pub(crate) fn process_mono(&mut self, input: f32) -> (f32, f32) {
        let target_azimuth = self.azimuth_target.get();
        let target_elevation = self.elevation_target.get();

        let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
        let smoothed_elevation = self.elevation_smoother.process(target_elevation);

        // Convert azimuth to radians for calculations
        // Note: We use a simple left/right model, elevation affects overall level
        let azimuth_rad = smoothed_azimuth.to_radians();

        // Calculate ITD (Interaural Time Difference)
        // Woodworth-Schlosberg formula (simplified):
        // ITD ≈ (head_radius / speed_of_sound) * (azimuth + sin(azimuth))
        // Max ITD is about 0.66ms for human head
        const HEAD_RADIUS: f32 = 0.0875; // ~8.75cm average head radius
        const SPEED_OF_SOUND: f32 = 343.0; // m/s
        let max_itd_seconds = HEAD_RADIUS / SPEED_OF_SOUND;
        let itd_factor = (azimuth_rad + azimuth_rad.sin()) / core::f32::consts::PI;
        let itd_seconds = max_itd_seconds * itd_factor;
        let itd_samples = (itd_seconds * self.sample_rate).round() as i32;

        // Calculate ILD (Interaural Level Difference)
        // Simple frequency-independent model (real HRTF is frequency-dependent)
        // ILD increases with azimuth, max ~20dB at 90°
        let ild_db = (azimuth_rad.abs() / (core::f32::consts::PI / 2.0)) * 10.0; // Max 10dB attenuation
        let ild_linear = 10.0_f32.powf(-ild_db / 20.0);

        // Apply ILD (level difference)
        let (left_gain, right_gain) = if azimuth_rad > 0.0 {
            // Sound to the left: left ear louder, right ear quieter
            (1.0, ild_linear)
        } else {
            // Sound to the right: right ear louder, left ear quieter
            (ild_linear, 1.0)
        };

        // Apply elevation effect (simple model: high/low sounds are quieter)
        let elevation_factor = (1.0 - (smoothed_elevation.abs() / 90.0) * 0.3).max(0.7);
        let left_level = input * left_gain * elevation_factor;
        let right_level = input * right_gain * elevation_factor;

        // Apply ITD using simple delay
        // Write current sample to delay buffer
        self.delay_buffer_left[self.delay_write_pos] = left_level;
        self.delay_buffer_right[self.delay_write_pos] = right_level;

        // Read from delay buffer with ITD offset
        let buffer_len = self.delay_buffer_left.len();
        let left_delay_samples = if itd_samples > 0 {
            itd_samples as usize
        } else {
            0
        };
        let right_delay_samples = if itd_samples < 0 {
            (-itd_samples) as usize
        } else {
            0
        };

        let left_read_pos = (self.delay_write_pos + buffer_len - left_delay_samples) % buffer_len;
        let right_read_pos = (self.delay_write_pos + buffer_len - right_delay_samples) % buffer_len;

        let left_out = self.delay_buffer_left[left_read_pos];
        let right_out = self.delay_buffer_right[right_read_pos];

        // Advance write position
        self.delay_write_pos = (self.delay_write_pos + 1) % buffer_len;

        (left_out, right_out)
    }

    /// Process stereo input to binaural stereo output
    ///
    /// Pans left and right channels symmetrically around the specified position
    pub(crate) fn process_stereo(&mut self, left: f32, right: f32, width: f32) -> (f32, f32) {
        let width = width.clamp(0.0, 2.0);

        if width < 0.001 {
            // Mono: just process center
            let mono = (left + right) * 0.5;
            self.process_mono(mono)
        } else {
            // Stereo: pan L/R to offset positions
            let angle_offset = 15.0 * width; // ±15° gives good stereo imaging

            // Save original position
            let original_az = self.azimuth_target.get();
            let original_el = self.elevation_target.get();

            // Process left channel (offset left)
            self.set_position(original_az + angle_offset, original_el);
            let (l_left, l_right) = self.process_mono(left);

            // Process right channel (offset right)
            self.set_position(original_az - angle_offset, original_el);
            let (r_left, r_right) = self.process_mono(right);

            // Restore original position
            self.set_position(original_az, original_el);

            // Mix both channels
            ((l_left + r_left) * 0.5, (l_right + r_right) * 0.5)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binaural_panner_center() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(0.0, 0.0); // Front center

        // Process some samples to let smoothing converge
        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

        // At center, both channels should be roughly equal
        let (left, right) = panner.process_mono(1.0);
        assert!(
            (left - right).abs() < 0.2,
            "Center should have similar L/R levels"
        );
    }

    #[test]
    fn test_binaural_panner_left() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(90.0, 0.0); // Hard left

        // Process some samples to let smoothing and ITD buffers settle
        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

        // Left should be louder than right (ILD)
        let (left, right) = panner.process_mono(1.0);
        assert!(
            left > right,
            "Left channel should be louder for left position: L={} R={}",
            left,
            right
        );
    }

    #[test]
    fn test_binaural_panner_right() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(-90.0, 0.0); // Hard right

        // Process some samples to let smoothing and ITD buffers settle
        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

        // Right should be louder than left (ILD)
        let (left, right) = panner.process_mono(1.0);
        assert!(
            right > left,
            "Right channel should be louder for right position: L={} R={}",
            left,
            right
        );
    }

    #[test]
    fn test_binaural_panner_stereo() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(0.0, 0.0); // Center

        // Process some samples to settle
        (0..100).for_each(|_| {
            let _ = panner.process_stereo(1.0, 1.0, 1.0);
        });

        // Stereo processing should produce output
        let (left, right) = panner.process_stereo(1.0, 0.5, 1.0);
        assert!(
            left > 0.0 && right > 0.0,
            "Stereo processing should produce output"
        );
    }
}
