//! Loudness normalization utilities.
//!
//! Analysis functions are provided by tutti-core. This module provides
//! normalization functions that modify audio in-place.

use tutti_core::analyze_true_peak;

/// Apply loudness normalization (EBU R128).
///
/// # Arguments
/// * `left` - Left channel (modified in place)
/// * `right` - Right channel (modified in place)
/// * `current_lufs` - Current integrated loudness
/// * `target_lufs` - Target integrated loudness
/// * `true_peak_limit` - True peak ceiling in dBTP
pub fn normalize_loudness(
    left: &mut [f32],
    right: &mut [f32],
    current_lufs: f64,
    target_lufs: f64,
    true_peak_limit: f64,
) {
    let gain_db = target_lufs - current_lufs;
    let mut gain = 10.0_f64.powf(gain_db / 20.0) as f32;

    // Check if gain would exceed true peak limit
    let current_peak = analyze_true_peak(left, right);
    let new_peak = current_peak + gain_db;

    if new_peak > true_peak_limit {
        let reduction_db = new_peak - true_peak_limit;
        gain *= 10.0_f32.powf(-reduction_db as f32 / 20.0);
    }

    for i in 0..left.len() {
        left[i] *= gain;
        right[i] *= gain;
    }
}

/// Normalize audio to target peak level.
///
/// # Arguments
/// * `left` - Left channel (modified in place)
/// * `right` - Right channel (modified in place)
/// * `target_db` - Target peak level in dB (e.g., -0.1 for near-full-scale)
pub fn normalize_peak(left: &mut [f32], right: &mut [f32], target_db: f64) {
    let current_peak = analyze_true_peak(left, right);
    let gain_db = target_db - current_peak;
    let gain = 10.0_f64.powf(gain_db / 20.0) as f32;

    for sample in left.iter_mut() {
        *sample *= gain;
    }
    for sample in right.iter_mut() {
        *sample *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tutti_core::analyze_loudness;

    #[test]
    fn test_normalize_loudness() {
        let sample_rate = 44100;
        let duration_samples = sample_rate * 2;

        let mut left: Vec<f32> = (0..duration_samples)
            .map(|i| {
                0.1 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin()
            })
            .collect();
        let mut right = left.clone();

        let current = analyze_loudness(&left, &right, sample_rate as u32);
        normalize_loudness(&mut left, &mut right, current.integrated_lufs, -14.0, -1.0);

        let normalized = analyze_loudness(&left, &right, sample_rate as u32);

        assert!(
            (normalized.integrated_lufs - (-14.0)).abs() < 2.0,
            "Expected -14 LUFS, got {}",
            normalized.integrated_lufs
        );
    }
}
