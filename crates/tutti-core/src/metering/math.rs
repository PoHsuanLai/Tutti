//! DSP math utilities for metering.

/// Stereo buffer statistics for correlation/width analysis.
#[derive(Default)]
pub(crate) struct StereoStats {
    pub(crate) left_rms: f32,
    pub(crate) right_rms: f32,
    pub(crate) mid_rms: f32,
    pub(crate) side_rms: f32,
    pub(crate) correlation: f32,
}

impl StereoStats {
    pub(crate) fn compute(left: &[f32], right: &[f32]) -> Self {
        let len = left.len().min(right.len());
        if len == 0 {
            return Self::default();
        }

        let mut sum_l_sq = 0.0f64;
        let mut sum_r_sq = 0.0f64;
        let mut sum_lr = 0.0f64;
        let mut sum_mid_sq = 0.0f64;
        let mut sum_side_sq = 0.0f64;

        for i in 0..len {
            let l = left[i] as f64;
            let r = right[i] as f64;
            sum_l_sq += l * l;
            sum_r_sq += r * r;
            sum_lr += l * r;
            let mid = (l + r) * 0.5;
            let side = (l - r) * 0.5;
            sum_mid_sq += mid * mid;
            sum_side_sq += side * side;
        }

        let n = len as f64;
        let left_rms = (sum_l_sq / n).sqrt() as f32;
        let right_rms = (sum_r_sq / n).sqrt() as f32;
        let mid_rms = (sum_mid_sq / n).sqrt() as f32;
        let side_rms = (sum_side_sq / n).sqrt() as f32;

        let correlation = if sum_l_sq > 0.0 && sum_r_sq > 0.0 {
            (sum_lr / (sum_l_sq.sqrt() * sum_r_sq.sqrt())) as f32
        } else {
            0.0
        };

        Self {
            left_rms,
            right_rms,
            mid_rms,
            side_rms,
            correlation,
        }
    }

    #[inline]
    pub(crate) fn width(&self) -> f32 {
        1.0 - self.correlation
    }

    #[inline]
    pub(crate) fn balance(&self) -> f32 {
        let total = self.left_rms + self.right_rms;
        if total > 0.0 {
            (self.right_rms - self.left_rms) / total
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    extern crate alloc;
    use alloc::vec::Vec;

    #[test]
    fn test_stereo_stats_mono() {
        let buf = [0.5f32; 100];
        let stats = StereoStats::compute(&buf, &buf);
        assert!((stats.correlation - 1.0).abs() < 0.001);
        assert!(stats.width().abs() < 0.001);
    }

    #[test]
    fn test_stereo_stats_inverted() {
        let left = [0.5f32; 100];
        let right: Vec<f32> = left.iter().map(|&x| -x).collect();
        let stats = StereoStats::compute(&left, &right);
        assert!((stats.correlation + 1.0).abs() < 0.001);
        assert!((stats.width() - 2.0).abs() < 0.001);
    }
}
