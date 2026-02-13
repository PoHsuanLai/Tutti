//! Batch loudness analysis (EBU R128 / ITU-R BS.1770)
//!
//! Provides one-shot loudness measurement for offline analysis.
//! For real-time metering, use `MeteringManager` instead.

extern crate alloc;
use alloc::vec::Vec;
use ebur128::{EbuR128, Mode};

/// Result of loudness analysis.
#[derive(Debug, Clone, Copy)]
pub struct LoudnessResult {
    /// Integrated loudness in LUFS.
    pub integrated_lufs: f64,
    /// Maximum true peak in dBTP.
    pub true_peak_dbtp: f64,
    /// Loudness range in LU.
    pub loudness_range_lu: f64,
}

/// One-shot EBU R128 loudness analysis for offline processing.
/// For real-time metering, use `MeteringManager` instead.
pub fn analyze_loudness(left: &[f32], right: &[f32], sample_rate: u32) -> LoudnessResult {
    let mut meter = EbuR128::new(2, sample_rate, Mode::I | Mode::LRA | Mode::TRUE_PEAK)
        .expect("Failed to create EBU R128 meter");

    let len = left.len().min(right.len());
    if len > 0 {
        let frames_data: Vec<&[f32]> = vec![&left[..len], &right[..len]];
        let _ = meter.add_frames_planar_f32(&frames_data);
    }

    let integrated_lufs = meter.loudness_global().unwrap_or(-70.0);
    let loudness_range_lu = meter.loudness_range().unwrap_or(0.0);

    let true_peak_l = meter.true_peak(0).unwrap_or(0.0);
    let true_peak_r = meter.true_peak(1).unwrap_or(0.0);
    let true_peak_linear = true_peak_l.max(true_peak_r);

    let true_peak_dbtp = if true_peak_linear > 0.0 {
        20.0 * true_peak_linear.log10()
    } else {
        -144.0
    };

    LoudnessResult {
        integrated_lufs,
        true_peak_dbtp,
        loudness_range_lu,
    }
}

/// Returns true peak level in dBTP (uses 4x oversampling).
pub fn analyze_true_peak(left: &[f32], right: &[f32]) -> f64 {
    let mut meter =
        EbuR128::new(2, 48000, Mode::TRUE_PEAK).expect("Failed to create EBU R128 meter for peak");

    let len = left.len().min(right.len());
    if len > 0 {
        let frames_data: Vec<&[f32]> = vec![&left[..len], &right[..len]];
        let _ = meter.add_frames_planar_f32(&frames_data);
    }

    let true_peak_l = meter.true_peak(0).unwrap_or(0.0);
    let true_peak_r = meter.true_peak(1).unwrap_or(0.0);
    let true_peak_linear = true_peak_l.max(true_peak_r);

    if true_peak_linear > 0.0 {
        20.0 * true_peak_linear.log10()
    } else {
        -144.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_silence_loudness() {
        let left = vec![0.0f32; 44100];
        let right = vec![0.0f32; 44100];

        let result = analyze_loudness(&left, &right, 44100);
        assert!(result.integrated_lufs <= -70.0);
    }

    #[test]
    fn test_peak_calculation() {
        let left = vec![0.5, -0.8, 0.3];
        let right = vec![0.2, 0.9, -0.1];

        let peak = analyze_true_peak(&left, &right);
        assert!(peak > -2.0 && peak < 0.0, "Peak: {}", peak);
    }

    #[test]
    fn test_full_scale_sine() {
        let sample_rate = 44100;
        let duration = 4.0;
        let num_samples = (sample_rate as f64 * duration) as usize;

        let left: Vec<f32> = (0..num_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = left.clone();

        let result = analyze_loudness(&left, &right, sample_rate);

        assert!(
            result.integrated_lufs > -2.0 && result.integrated_lufs < 2.0,
            "Expected ~0 LUFS, got {}",
            result.integrated_lufs
        );
    }
}
