//! Lock-free amplitude metering.

use crate::AtomicFloat;

/// Lock-free amplitude storage (RMS L/R, Peak L/R).
pub struct AtomicAmplitude {
    rms_left: AtomicFloat,
    rms_right: AtomicFloat,
    peak_left: AtomicFloat,
    peak_right: AtomicFloat,
}

impl Default for AtomicAmplitude {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicAmplitude {
    pub fn new() -> Self {
        Self {
            rms_left: AtomicFloat::new(0.0),
            rms_right: AtomicFloat::new(0.0),
            peak_left: AtomicFloat::new(0.0),
            peak_right: AtomicFloat::new(0.0),
        }
    }

    #[inline]
    pub fn get(&self) -> (f32, f32, f32, f32) {
        (
            self.rms_left.get(),
            self.rms_right.get(),
            self.peak_left.get(),
            self.peak_right.get(),
        )
    }

    #[inline]
    pub fn set(&self, rms_l: f32, rms_r: f32, peak_l: f32, peak_r: f32) {
        self.rms_left.set(rms_l);
        self.rms_right.set(rms_r);
        self.peak_left.set(peak_l);
        self.peak_right.set(peak_r);
    }
}
