//! Smoothed parameter values for zipper-free automation.
//!
//! Provides per-sample smoothing to avoid audible artifacts when parameters change abruptly.
//!
//! # Example
//!
//! ```
//! use tutti_core::SmoothedValue;
//!
//! // Create with 10ms smoothing at 44.1kHz
//! let mut gain = SmoothedValue::new(1.0, 0.010, 44100.0);
//!
//! // When automation changes target
//! gain.set_target(0.5);
//!
//! // In audio callback (per-sample)
//! # let mut buffer = [0.0f32; 512];
//! for sample in buffer.iter_mut() {
//!     *sample *= gain.next_sample();
//! }
//! ```

/// Smoothed parameter value for zipper-free automation.
///
/// Uses linear interpolation to smoothly transition from current value to target
/// over a configurable time period. Call [`next_sample()`](SmoothedValue::next_sample) once per sample
/// in the audio callback.
#[derive(Debug, Clone)]
pub struct SmoothedValue {
    current: f32,
    target: f32,
    step: f32,
    samples_remaining: u32,
    smooth_samples: u32,
}

impl SmoothedValue {
    pub fn new(initial: f32, smooth_time_secs: f32, sample_rate: f32) -> Self {
        let smooth_samples = (smooth_time_secs * sample_rate).max(1.0) as u32;

        Self {
            current: initial,
            target: initial,
            step: 0.0,
            samples_remaining: 0,
            smooth_samples,
        }
    }

    pub fn immediate(initial: f32) -> Self {
        Self {
            current: initial,
            target: initial,
            step: 0.0,
            samples_remaining: 0,
            smooth_samples: 1,
        }
    }

    #[inline]
    pub fn set_target(&mut self, target: f32) {
        if (target - self.target).abs() < f32::EPSILON {
            return; // No change
        }

        self.target = target;
        self.samples_remaining = self.smooth_samples;

        if self.samples_remaining > 0 {
            self.step = (self.target - self.current) / self.samples_remaining as f32;
        }
    }

    #[inline]
    pub fn set_immediate(&mut self, value: f32) {
        self.current = value;
        self.target = value;
        self.step = 0.0;
        self.samples_remaining = 0;
    }

    /// Call once per sample in the audio callback.
    #[inline]
    pub fn next_sample(&mut self) -> f32 {
        if self.samples_remaining > 0 {
            self.current += self.step;
            self.samples_remaining -= 1;

            // Snap to target when done to avoid floating point drift
            if self.samples_remaining == 0 {
                self.current = self.target;
            }
        }

        self.current
    }

    #[inline]
    pub fn current(&self) -> f32 {
        self.current
    }

    #[inline]
    pub fn target(&self) -> f32 {
        self.target
    }

    #[inline]
    pub fn is_smoothing(&self) -> bool {
        self.samples_remaining > 0
    }

    #[inline]
    pub fn samples_remaining(&self) -> u32 {
        self.samples_remaining
    }

    /// Takes effect on the next `set_target()` call.
    pub fn set_smooth_time(&mut self, smooth_time_secs: f32, sample_rate: f32) {
        self.smooth_samples = (smooth_time_secs * sample_rate).max(1.0) as u32;
    }

    #[inline]
    pub fn skip_to_target(&mut self) {
        self.current = self.target;
        self.step = 0.0;
        self.samples_remaining = 0;
    }

    #[inline]
    pub fn process_block(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.next_sample();
        }
    }

    #[inline]
    pub fn apply_gain(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample *= self.next_sample();
        }
    }
}

impl Default for SmoothedValue {
    fn default() -> Self {
        Self::new(0.0, 0.005, 44100.0) // 5ms default at 44.1kHz
    }
}

/// Stereo pair of smoothed values.
///
/// Convenient for panning or stereo gain automation.
#[derive(Debug, Clone)]
pub struct SmoothedStereo {
    pub left: SmoothedValue,
    pub right: SmoothedValue,
}

impl SmoothedStereo {
    pub fn new(
        initial_left: f32,
        initial_right: f32,
        smooth_time_secs: f32,
        sample_rate: f32,
    ) -> Self {
        Self {
            left: SmoothedValue::new(initial_left, smooth_time_secs, sample_rate),
            right: SmoothedValue::new(initial_right, smooth_time_secs, sample_rate),
        }
    }

    pub fn mono(initial: f32, smooth_time_secs: f32, sample_rate: f32) -> Self {
        Self::new(initial, initial, smooth_time_secs, sample_rate)
    }

    pub fn set_targets(&mut self, left: f32, right: f32) {
        self.left.set_target(left);
        self.right.set_target(right);
    }

    pub fn set_target_mono(&mut self, value: f32) {
        self.left.set_target(value);
        self.right.set_target(value);
    }

    #[inline]
    pub fn next_sample(&mut self) -> (f32, f32) {
        (self.left.next_sample(), self.right.next_sample())
    }

    pub fn is_smoothing(&self) -> bool {
        self.left.is_smoothing() || self.right.is_smoothing()
    }

    pub fn apply_gain_stereo(&mut self, left_buf: &mut [f32], right_buf: &mut [f32]) {
        let len = left_buf.len().min(right_buf.len());
        for i in 0..len {
            left_buf[i] *= self.left.next_sample();
            right_buf[i] *= self.right.next_sample();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 0.0001;

    fn approx_eq(a: f32, b: f32) -> bool {
        (a - b).abs() < EPSILON
    }

    #[test]
    fn test_immediate_value() {
        let smooth = SmoothedValue::new(1.0, 0.010, 44100.0);

        assert!(approx_eq(smooth.current(), 1.0));
        assert!(approx_eq(smooth.target(), 1.0));
        assert!(!smooth.is_smoothing());
    }

    #[test]
    fn test_set_target_starts_smoothing() {
        let mut smooth = SmoothedValue::new(0.0, 0.010, 44100.0);
        smooth.set_target(1.0);

        assert!(smooth.is_smoothing());
        assert!(approx_eq(smooth.target(), 1.0));
        assert!(smooth.samples_remaining() > 0);
    }

    #[test]
    fn test_smoothing_reaches_target() {
        let sample_rate = 44100.0;
        let smooth_time = 0.001; // 1ms = ~44 samples
        let mut smooth = SmoothedValue::new(0.0, smooth_time, sample_rate);

        smooth.set_target(1.0);

        let expected_samples = (smooth_time * sample_rate) as usize;

        // Process enough samples to reach target
        for _ in 0..expected_samples + 10 {
            smooth.next_sample();
        }

        assert!(!smooth.is_smoothing());
        assert!(approx_eq(smooth.current(), 1.0));
    }

    #[test]
    fn test_set_immediate() {
        let mut smooth = SmoothedValue::new(0.0, 0.010, 44100.0);
        smooth.set_target(0.5); // Start smoothing

        smooth.set_immediate(1.0);

        assert!(!smooth.is_smoothing());
        assert!(approx_eq(smooth.current(), 1.0));
        assert!(approx_eq(smooth.target(), 1.0));
    }

    #[test]
    fn test_skip_to_target() {
        let mut smooth = SmoothedValue::new(0.0, 0.010, 44100.0);
        smooth.set_target(1.0);

        assert!(smooth.is_smoothing());

        smooth.skip_to_target();

        assert!(!smooth.is_smoothing());
        assert!(approx_eq(smooth.current(), 1.0));
    }

    #[test]
    fn test_process_block() {
        let mut smooth = SmoothedValue::new(0.0, 0.001, 1000.0); // 1ms at 1kHz = 1 sample
        smooth.set_target(1.0);

        let mut buffer = [0.0f32; 10];
        smooth.process_block(&mut buffer);

        // First value should be stepped toward target
        assert!(buffer[0] > 0.0);
        // Last value should be at or near target
        assert!(approx_eq(buffer[9], 1.0) || buffer[9] > 0.9);
    }

    #[test]
    fn test_apply_gain() {
        let mut smooth = SmoothedValue::immediate(0.5);

        let mut buffer = [1.0f32; 4];
        smooth.apply_gain(&mut buffer);

        for sample in buffer.iter() {
            assert!(approx_eq(*sample, 0.5));
        }
    }

    #[test]
    fn test_smoothed_stereo() {
        let mut stereo = SmoothedStereo::mono(1.0, 0.001, 1000.0);

        stereo.set_targets(0.5, 0.8);

        assert!(stereo.is_smoothing());

        // Process until done
        for _ in 0..10 {
            stereo.next_sample();
        }

        assert!(approx_eq(stereo.left.current(), 0.5));
        assert!(approx_eq(stereo.right.current(), 0.8));
    }

    #[test]
    fn test_retarget_while_smoothing() {
        let mut smooth = SmoothedValue::new(0.0, 0.010, 44100.0);

        smooth.set_target(1.0);

        // Advance partway
        for _ in 0..100 {
            smooth.next_sample();
        }

        let mid_value = smooth.current();
        assert!(mid_value > 0.0 && mid_value < 1.0);

        // Change target mid-smooth
        smooth.set_target(0.0);

        assert!(smooth.is_smoothing());
        assert!(approx_eq(smooth.target(), 0.0));

        // Should now be heading toward 0
        for _ in 0..1000 {
            smooth.next_sample();
        }

        assert!(approx_eq(smooth.current(), 0.0));
    }

    #[test]
    fn test_no_change_if_same_target() {
        let mut smooth = SmoothedValue::new(0.5, 0.010, 44100.0);

        smooth.set_target(0.5); // Same as current

        assert!(!smooth.is_smoothing());
    }
}
