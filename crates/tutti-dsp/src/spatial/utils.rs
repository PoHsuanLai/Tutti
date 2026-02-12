//! Internal utilities for spatial audio processing

/// Default smoothing time for position changes (50ms for smooth automation)
pub(crate) const DEFAULT_POSITION_SMOOTH_TIME: f32 = 0.05;

/// Simple exponential smoothing filter for real-time parameter smoothing.
pub(crate) struct ExponentialSmoother {
    value: f32,
    coeff: f32,
}

impl ExponentialSmoother {
    /// Create a new smoother with smoothing time in seconds.
    pub fn new(smooth_time: f32, sample_rate: f32) -> Self {
        let coeff = 1.0 - (-1.0 / (smooth_time * sample_rate)).exp();
        Self {
            value: 0.0,
            coeff: coeff.clamp(0.0, 1.0),
        }
    }

    /// Process one sample (returns smoothed value)
    #[inline]
    pub fn process(&mut self, target: f32) -> f32 {
        self.value += self.coeff * (target - self.value);
        self.value
    }

    /// Reset to a specific value
    #[allow(dead_code)]
    pub fn reset(&mut self, value: f32) {
        self.value = value;
    }
}
