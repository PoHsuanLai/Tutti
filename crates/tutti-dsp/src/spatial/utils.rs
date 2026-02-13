/// 50ms smoothing for position changes
pub(crate) const DEFAULT_POSITION_SMOOTH_TIME: f32 = 0.05;

pub(crate) struct ExponentialSmoother {
    value: f32,
    coeff: f32,
}

impl ExponentialSmoother {
    pub fn new(smooth_time: f32, sample_rate: f32) -> Self {
        let coeff = 1.0 - (-1.0 / (smooth_time * sample_rate)).exp();
        Self {
            value: 0.0,
            coeff: coeff.clamp(0.0, 1.0),
        }
    }

    #[inline]
    pub fn process(&mut self, target: f32) -> f32 {
        self.value += self.coeff * (target - self.value);
        self.value
    }

    #[allow(dead_code)]
    pub fn reset(&mut self, value: f32) {
        self.value = value;
    }
}
