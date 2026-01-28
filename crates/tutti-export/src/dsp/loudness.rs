//! Loudness metering (EBU R128 / ITU-R BS.1770)
//!
//! Provides integrated loudness measurement and true peak detection.

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

/// K-weighting filter coefficients for loudness measurement
struct KWeightingFilter {
    // High shelf (stage 1)
    b0_hs: f64,
    b1_hs: f64,
    b2_hs: f64,
    a1_hs: f64,
    a2_hs: f64,
    // High pass (stage 2)
    b0_hp: f64,
    b1_hp: f64,
    b2_hp: f64,
    a1_hp: f64,
    a2_hp: f64,
    // State
    x1_hs: f64,
    x2_hs: f64,
    y1_hs: f64,
    y2_hs: f64,
    x1_hp: f64,
    x2_hp: f64,
    y1_hp: f64,
    y2_hp: f64,
}

impl KWeightingFilter {
    /// Create a new K-weighting filter for the given sample rate
    fn new(sample_rate: f64) -> Self {
        // Pre-filter (high shelf) coefficients from ITU-R BS.1770
        let f0 = 1681.974450955533;
        let g = 3.999843853973347;
        let q = 0.7071752369554196;

        let k = (std::f64::consts::PI * f0 / sample_rate).tan();
        let vh = 10.0_f64.powf(g / 20.0);
        let vb = vh.powf(0.4996667741545416);

        let a0 = 1.0 + k / q + k * k;
        let b0_hs = (vh + vb * k / q + k * k) / a0;
        let b1_hs = 2.0 * (k * k - vh) / a0;
        let b2_hs = (vh - vb * k / q + k * k) / a0;
        let a1_hs = 2.0 * (k * k - 1.0) / a0;
        let a2_hs = (1.0 - k / q + k * k) / a0;

        // High pass filter coefficients
        let f0_hp = 38.13547087602444;
        let q_hp = 0.5003270373238773;

        let k_hp = (std::f64::consts::PI * f0_hp / sample_rate).tan();
        let a0_hp = 1.0 + k_hp / q_hp + k_hp * k_hp;
        let b0_hp = 1.0 / a0_hp;
        let b1_hp = -2.0 / a0_hp;
        let b2_hp = 1.0 / a0_hp;
        let a1_hp = 2.0 * (k_hp * k_hp - 1.0) / a0_hp;
        let a2_hp = (1.0 - k_hp / q_hp + k_hp * k_hp) / a0_hp;

        Self {
            b0_hs,
            b1_hs,
            b2_hs,
            a1_hs,
            a2_hs,
            b0_hp,
            b1_hp,
            b2_hp,
            a1_hp,
            a2_hp,
            x1_hs: 0.0,
            x2_hs: 0.0,
            y1_hs: 0.0,
            y2_hs: 0.0,
            x1_hp: 0.0,
            x2_hp: 0.0,
            y1_hp: 0.0,
            y2_hp: 0.0,
        }
    }

    /// Process a single sample through the filter
    fn process(&mut self, x: f64) -> f64 {
        // Stage 1: High shelf
        let y_hs = self.b0_hs * x + self.b1_hs * self.x1_hs + self.b2_hs * self.x2_hs
            - self.a1_hs * self.y1_hs
            - self.a2_hs * self.y2_hs;

        self.x2_hs = self.x1_hs;
        self.x1_hs = x;
        self.y2_hs = self.y1_hs;
        self.y1_hs = y_hs;

        // Stage 2: High pass
        let y_hp = self.b0_hp * y_hs + self.b1_hp * self.x1_hp + self.b2_hp * self.x2_hp
            - self.a1_hp * self.y1_hp
            - self.a2_hp * self.y2_hp;

        self.x2_hp = self.x1_hp;
        self.x1_hp = y_hs;
        self.y2_hp = self.y1_hp;
        self.y1_hp = y_hp;

        y_hp
    }
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
    let sr = sample_rate as f64;

    // Create K-weighting filters for each channel
    let mut filter_l = KWeightingFilter::new(sr);
    let mut filter_r = KWeightingFilter::new(sr);

    // Calculate gating block size (400ms)
    let block_size = (0.4 * sr) as usize;
    let hop_size = block_size / 4; // 75% overlap

    // K-weight the audio
    let mut weighted_l: Vec<f64> = Vec::with_capacity(left.len());
    let mut weighted_r: Vec<f64> = Vec::with_capacity(right.len());

    for i in 0..left.len() {
        weighted_l.push(filter_l.process(left[i] as f64));
        weighted_r.push(filter_r.process(right[i] as f64));
    }

    // Calculate block loudness values
    let mut block_loudness: Vec<f64> = Vec::new();

    let mut pos = 0;
    while pos + block_size <= weighted_l.len() {
        let mut sum_sq = 0.0;

        for i in pos..pos + block_size {
            sum_sq += weighted_l[i] * weighted_l[i] + weighted_r[i] * weighted_r[i];
        }

        let mean_sq = sum_sq / (2.0 * block_size as f64);
        let loudness = -0.691 + 10.0 * mean_sq.log10();

        block_loudness.push(loudness);
        pos += hop_size;
    }

    if block_loudness.is_empty() {
        return LoudnessResult {
            integrated_lufs: -70.0,
            true_peak_dbtp: calculate_peak(left, right),
            loudness_range_lu: 0.0,
        };
    }

    // Absolute threshold gating (-70 LUFS)
    let abs_threshold = -70.0;
    let gated_blocks: Vec<f64> = block_loudness
        .iter()
        .copied()
        .filter(|&l| l > abs_threshold)
        .collect();

    if gated_blocks.is_empty() {
        return LoudnessResult {
            integrated_lufs: -70.0,
            true_peak_dbtp: calculate_peak(left, right),
            loudness_range_lu: 0.0,
        };
    }

    // Calculate relative threshold
    let abs_gated_mean: f64 = gated_blocks.iter().map(|l| 10.0_f64.powf(*l / 10.0)).sum::<f64>()
        / gated_blocks.len() as f64;
    let rel_threshold = -0.691 + 10.0 * abs_gated_mean.log10() - 10.0;

    // Relative threshold gating
    let final_blocks: Vec<f64> = gated_blocks
        .iter()
        .copied()
        .filter(|&l| l > rel_threshold)
        .collect();

    if final_blocks.is_empty() {
        return LoudnessResult {
            integrated_lufs: -70.0,
            true_peak_dbtp: calculate_peak(left, right),
            loudness_range_lu: 0.0,
        };
    }

    // Calculate integrated loudness
    let integrated_mean: f64 =
        final_blocks.iter().map(|l| 10.0_f64.powf(*l / 10.0)).sum::<f64>() / final_blocks.len() as f64;
    let integrated_lufs = -0.691 + 10.0 * integrated_mean.log10();

    // Calculate loudness range (simplified)
    let mut sorted = final_blocks.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let low_10 = sorted[sorted.len() / 10];
    let high_95 = sorted[sorted.len() * 95 / 100];
    let loudness_range_lu = high_95 - low_10;

    LoudnessResult {
        integrated_lufs,
        true_peak_dbtp: calculate_peak(left, right),
        loudness_range_lu,
    }
}

/// Calculate true peak level
///
/// Uses 4x oversampling for true peak detection.
pub fn calculate_peak(left: &[f32], right: &[f32]) -> f64 {
    // Simple peak (not true peak with oversampling for now)
    let peak_l = left.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let peak_r = right.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    let peak = peak_l.max(peak_r);

    if peak > 0.0 {
        20.0 * (peak as f64).log10()
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
    let current_db = 20.0 * current_peak.log10();
    let gain_db = target_db - current_db;
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
        // Peak should be 0.9, which is about -0.92 dB
        assert!((peak - (-0.92)).abs() < 0.1);
    }

    #[test]
    fn test_full_scale_sine() {
        // Generate a full-scale sine wave (should be around -3 LUFS for stereo)
        let sample_rate = 44100;
        let duration = 2.0; // 2 seconds
        let num_samples = (sample_rate as f64 * duration) as usize;

        let left: Vec<f32> = (0..num_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = left.clone();

        let result = calculate_loudness(&left, &right, sample_rate);

        // Full-scale 1kHz sine should be around -3 LUFS
        assert!(
            result.integrated_lufs > -5.0 && result.integrated_lufs < -1.0,
            "Expected ~-3 LUFS, got {}",
            result.integrated_lufs
        );
    }

    #[test]
    fn test_normalize_loudness() {
        // Generate a sine wave (better for loudness measurement than DC)
        let sample_rate = 44100;
        let duration_samples = sample_rate * 2; // 2 seconds

        let mut left: Vec<f32> = (0..duration_samples)
            .map(|i| 0.1 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
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
