//! Internal utilities for spatial audio processing

/// Default smoothing time for position changes (50ms for smooth automation)
pub(crate) const DEFAULT_POSITION_SMOOTH_TIME: f32 = 0.05;

/// Simple exponential smoothing filter for real-time parameter smoothing
///
/// Used internally for smooth position transitions in spatial panners.
pub(crate) struct ExponentialSmoother {
    /// Current smoothed value
    value: f32,
    /// Smoothing coefficient (0 = no smoothing, 1 = instant)
    coeff: f32,
}

impl ExponentialSmoother {
    /// Create a new smoother with smoothing time in seconds
    ///
    /// At 48kHz, 0.05s = 2400 samples. We want to reach ~99% in that time.
    /// tau = -1 / ln(1 - target_level), for 99% convergence: tau ≈ 4.6
    /// coeff = 1 - exp(-1 / (time * sample_rate))
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
