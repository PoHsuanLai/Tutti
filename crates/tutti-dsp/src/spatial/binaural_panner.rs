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
    azimuth_target: Arc<AtomicFloat>,
    elevation_target: Arc<AtomicFloat>,
    azimuth_smoother: ExponentialSmoother,
    elevation_smoother: ExponentialSmoother,
    sample_rate: f32,
    delay_buffer_left: Vec<f32>,
    delay_buffer_right: Vec<f32>,
    delay_write_pos: usize,
}

impl BinauralPanner {
    /// Create a new binaural panner.
    pub(crate) fn new(sample_rate: f32) -> Self {
        const MAX_ITD_SAMPLES: usize = 64;

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

    /// Process mono input to binaural stereo output using ITD/ILD model.
    pub(crate) fn process_mono(&mut self, input: f32) -> (f32, f32) {
        let target_azimuth = self.azimuth_target.get();
        let target_elevation = self.elevation_target.get();

        let smoothed_azimuth = self.azimuth_smoother.process(target_azimuth);
        let smoothed_elevation = self.elevation_smoother.process(target_elevation);

        let azimuth_rad = smoothed_azimuth.to_radians();

        // Woodworth-Schlosberg ITD formula
        const HEAD_RADIUS: f32 = 0.0875;
        const SPEED_OF_SOUND: f32 = 343.0;
        let max_itd_seconds = HEAD_RADIUS / SPEED_OF_SOUND;
        let itd_factor = (azimuth_rad + azimuth_rad.sin()) / core::f32::consts::PI;
        let itd_seconds = max_itd_seconds * itd_factor;
        let itd_samples = (itd_seconds * self.sample_rate).round() as i32;

        let ild_db = (azimuth_rad.abs() / (core::f32::consts::PI / 2.0)) * 10.0;
        let ild_linear = 10.0_f32.powf(-ild_db / 20.0);

        let (left_gain, right_gain) = if azimuth_rad > 0.0 {
            (1.0, ild_linear)
        } else {
            (ild_linear, 1.0)
        };

        let elevation_factor = (1.0 - (smoothed_elevation.abs() / 90.0) * 0.3).max(0.7);
        let left_level = input * left_gain * elevation_factor;
        let right_level = input * right_gain * elevation_factor;

        self.delay_buffer_left[self.delay_write_pos] = left_level;
        self.delay_buffer_right[self.delay_write_pos] = right_level;

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

        self.delay_write_pos = (self.delay_write_pos + 1) % buffer_len;

        (left_out, right_out)
    }

    /// Process stereo input to binaural stereo output.
    pub(crate) fn process_stereo(&mut self, left: f32, right: f32, width: f32) -> (f32, f32) {
        let width = width.clamp(0.0, 2.0);

        if width < 0.001 {
            let mono = (left + right) * 0.5;
            self.process_mono(mono)
        } else {
            let angle_offset = 15.0 * width;

            let original_az = self.azimuth_target.get();
            let original_el = self.elevation_target.get();

            self.set_position(original_az + angle_offset, original_el);
            let (l_left, l_right) = self.process_mono(left);

            self.set_position(original_az - angle_offset, original_el);
            let (r_left, r_right) = self.process_mono(right);

            self.set_position(original_az, original_el);

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
        panner.set_position(0.0, 0.0);

        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

        let (left, right) = panner.process_mono(1.0);
        assert!((left - right).abs() < 0.2);
    }

    #[test]
    fn test_binaural_panner_left() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(90.0, 0.0);

        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

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
        panner.set_position(-90.0, 0.0);

        (0..100).for_each(|_| {
            let _ = panner.process_mono(1.0);
        });

        let (left, right) = panner.process_mono(1.0);
        assert!(
            right > left,
            "Right channel should be louder for right position: L={} R={}",
            left,
            right
        );
    }

    #[test]
    fn test_binaural_panner_stereo_preserves_asymmetry() {
        let mut panner = BinauralPanner::new(48000.0);
        panner.set_position(0.0, 0.0);

        (0..100).for_each(|_| {
            let _ = panner.process_stereo(1.0, 0.5, 1.0);
        });

        let (left, right) = panner.process_stereo(1.0, 0.5, 1.0);
        assert!(
            left > right,
            "Stereo with louder left input should produce louder left output: L={} R={}",
            left,
            right
        );
    }
}
