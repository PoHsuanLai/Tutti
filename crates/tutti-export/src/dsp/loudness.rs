//! Loudness metering (EBU R128 / ITU-R BS.1770)
//!
//! Provides integrated loudness measurement and true peak detection using the ebur128 crate.

use ebur128::{EbuR128, Mode};

/// Result of loudness analysis
#[derive(Debug, Clone, Copy)]
pub struct LoudnessResult {
    /// Integrated loudness in LUFS
    pub integrated_lufs: f64,
    /// Maximum true peak in dBTP
    pub true_peak_dbtp: f64,
    /// Loudness range in LU
    pub loudness_range_lu: f64,
}

/// Calculate integrated loudness (EBU R128)
///
/// # Arguments
/// * `left` - Left channel samples
/// * `right` - Right channel samples
/// * `sample_rate` - Sample rate in Hz
///
/// # Returns
/// Loudness measurement result
pub fn calculate_loudness(left: &[f32], right: &[f32], sample_rate: u32) -> LoudnessResult {
    // Create EBU R128 meter with all required modes
    let mut meter = EbuR128::new(
        2, // stereo
        sample_rate,
        Mode::I | Mode::LRA | Mode::TRUE_PEAK,
    )
    .expect("Failed to create EBU R128 meter");

    // Add frames to the meter using planar format (separate left/right channels)
    let len = left.len().min(right.len());
    if len > 0 {
        let frames_data: Vec<&[f32]> = vec![&left[..len], &right[..len]];
        let _ = meter.add_frames_planar_f32(&frames_data);
    }

    // Get integrated loudness
    let integrated_lufs = meter.loudness_global().unwrap_or(-70.0);

    // Get loudness range
    let loudness_range_lu = meter.loudness_range().unwrap_or(0.0);

    // Get true peak for both channels (returns linear values)
    let true_peak_l = meter.true_peak(0).unwrap_or(0.0);
    let true_peak_r = meter.true_peak(1).unwrap_or(0.0);
    let true_peak_linear = true_peak_l.max(true_peak_r);

    // Convert to dBTP
    let true_peak_dbtp = if true_peak_linear > 0.0 {
        20.0 * true_peak_linear.log10()
    } else {
        -144.0 // Approximately -infinity dB
    };

    LoudnessResult {
        integrated_lufs,
        true_peak_dbtp,
        loudness_range_lu,
    }
}

/// Calculate true peak level
///
/// Uses 4x oversampling for true peak detection via ebur128.
pub fn calculate_peak(left: &[f32], right: &[f32]) -> f64 {
    // Create a simple meter just for true peak
    let mut meter = EbuR128::new(
        2,     // stereo
        48000, // sample rate doesn't affect true peak measurement significantly
        Mode::TRUE_PEAK,
    )
    .expect("Failed to create EBU R128 meter for peak");

    // Add frames using planar format
    let len = left.len().min(right.len());
    if len > 0 {
        let frames_data: Vec<&[f32]> = vec![&left[..len], &right[..len]];
        let _ = meter.add_frames_planar_f32(&frames_data);
    }

    // Get true peak for both channels (returns linear values)
    let true_peak_l = meter.true_peak(0).unwrap_or(0.0);
    let true_peak_r = meter.true_peak(1).unwrap_or(0.0);
    let true_peak_linear = true_peak_l.max(true_peak_r);

    // Convert to dBTP
    if true_peak_linear > 0.0 {
        20.0 * true_peak_linear.log10()
    } else {
        -144.0 // Approximately -infinity dB
    }
}

/// Apply loudness normalization
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
    // Calculate gain needed
    let gain_db = target_lufs - current_lufs;
    let mut gain = 10.0_f64.powf(gain_db / 20.0) as f32;

    // Check if gain would exceed true peak limit
    let current_peak = calculate_peak(left, right);
    let new_peak = current_peak + gain_db;

    if new_peak > true_peak_limit {
        // Reduce gain to stay within limit
        let reduction_db = new_peak - true_peak_limit;
        gain *= 10.0_f32.powf(-reduction_db as f32 / 20.0);
    }

    // Apply gain
    for i in 0..left.len() {
        left[i] *= gain;
        right[i] *= gain;
    }
}

/// Normalize audio to target peak level
///
/// # Arguments
/// * `left` - Left channel (modified in place)
/// * `right` - Right channel (modified in place)
/// * `target_db` - Target peak level in dB (e.g., -0.1 for near-full-scale)
pub fn normalize_peak(left: &mut [f32], right: &mut [f32], target_db: f64) {
    let current_peak = calculate_peak(left, right);
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

    #[test]
    fn test_silence_loudness() {
        let left = vec![0.0f32; 44100];
        let right = vec![0.0f32; 44100];

        let result = calculate_loudness(&left, &right, 44100);
        assert!(result.integrated_lufs <= -70.0);
    }

    #[test]
    fn test_peak_calculation() {
        let left = vec![0.5, -0.8, 0.3];
        let right = vec![0.2, 0.9, -0.1];

        let peak = calculate_peak(&left, &right);
        // Peak should be close to 0.9, which is about -0.92 dB
        // ebur128 uses true peak with oversampling, so it might be slightly different
        assert!(peak > -2.0 && peak < 0.0, "Peak: {}", peak);
    }

    #[test]
    fn test_full_scale_sine() {
        // Generate a full-scale sine wave (should be around -3 LUFS for stereo)
        let sample_rate = 44100;
        let duration = 4.0; // 4 seconds (need enough for EBU R128 gating blocks)
        let num_samples = (sample_rate as f64 * duration) as usize;

        let left: Vec<f32> = (0..num_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = left.clone();

        let result = calculate_loudness(&left, &right, sample_rate);

        // Full-scale 1kHz sine should be close to 0 LUFS (not -3 dBFS!)
        // EBU R128 calibrates 0 LUFS to be at -23 dBFS RMS for broadcast level,
        // but a full-scale sine wave is at 0 dBFS peak, which is about 0 LUFS
        assert!(
            result.integrated_lufs > -2.0 && result.integrated_lufs < 2.0,
            "Expected ~0 LUFS, got {}",
            result.integrated_lufs
        );
    }

    #[test]
    fn test_normalize_loudness() {
        // Generate a sine wave (better for loudness measurement than DC)
        let sample_rate = 44100;
        let duration_samples = sample_rate * 2; // 2 seconds

        let mut left: Vec<f32> = (0..duration_samples)
            .map(|i| {
                0.1 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin()
            })
            .collect();
        let mut right = left.clone();

        let current = calculate_loudness(&left, &right, sample_rate as u32);
        normalize_loudness(&mut left, &mut right, current.integrated_lufs, -14.0, -1.0);

        let normalized = calculate_loudness(&left, &right, sample_rate as u32);

        // Should be close to -14 LUFS (within 2 LU due to measurement variability)
        assert!(
            (normalized.integrated_lufs - (-14.0)).abs() < 2.0,
            "Expected -14 LUFS, got {}",
            normalized.integrated_lufs
        );
    }
}
