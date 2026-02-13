//! Stereo correlation and width analysis.

use super::math::StereoStats;
use crate::AtomicFloat;

/// Lock-free stereo analysis storage.
pub struct AtomicStereoAnalysis {
    correlation: AtomicFloat,
    width: AtomicFloat,
    balance: AtomicFloat,
    mid_level: AtomicFloat,
    side_level: AtomicFloat,
}

impl Default for AtomicStereoAnalysis {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomicStereoAnalysis {
    pub fn new() -> Self {
        Self {
            correlation: AtomicFloat::new(0.0),
            width: AtomicFloat::new(1.0),
            balance: AtomicFloat::new(0.0),
            mid_level: AtomicFloat::new(0.0),
            side_level: AtomicFloat::new(0.0),
        }
    }

    #[inline]
    pub fn get(&self) -> StereoAnalysisSnapshot {
        StereoAnalysisSnapshot {
            correlation: self.correlation.get(),
            width: self.width.get(),
            balance: self.balance.get(),
            mid_level: self.mid_level.get(),
            side_level: self.side_level.get(),
        }
    }

    #[inline]
    pub fn set(&self, correlation: f32, width: f32, balance: f32, mid_level: f32, side_level: f32) {
        self.correlation.set(correlation);
        self.width.set(width);
        self.balance.set(balance);
        self.mid_level.set(mid_level);
        self.side_level.set(side_level);
    }

    pub fn update_from_buffers(&self, left: &[f32], right: &[f32]) {
        let stats = StereoStats::compute(left, right);
        self.set(
            stats.correlation,
            stats.width(),
            stats.balance(),
            stats.mid_rms,
            stats.side_rms,
        );
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StereoAnalysisSnapshot {
    pub correlation: f32,
    pub width: f32,
    pub balance: f32,
    pub mid_level: f32,
    pub side_level: f32,
}
