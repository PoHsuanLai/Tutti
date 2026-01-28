//! Stereo Correlation and Phase Analysis
//!
//! Provides analysis tools for stereo audio:
//! - **Correlation**: Phase coherence between L/R channels (-1 to +1)
//! - **Stereo width**: How "wide" the stereo image is
//! - **Balance**: Left/right balance
//! - **Mid/Side levels**: M/S encoding analysis
//!
//! ## Use Cases
//!
//! - Phase correlation meters (prevent mono compatibility issues)
//! - Stereo width visualization
//! - Mixing feedback
//! - Mastering analysis

/// Stereo analysis results
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct StereoAnalysis {
    /// Phase correlation (-1.0 to 1.0)
    /// - 1.0 = Mono (L and R identical)
    /// - 0.0 = Uncorrelated (independent L/R)
    /// - -1.0 = Out of phase (L and R are inverted)
    pub correlation: f32,

    /// Stereo width (0.0 to 2.0)
    /// - 0.0 = Mono
    /// - 1.0 = Normal stereo
    /// - 2.0 = Maximum width (out of phase)
    pub width: f32,

    /// Left/right balance (-1.0 to 1.0)
    /// - -1.0 = Full left
    /// - 0.0 = Center
    /// - 1.0 = Full right
    pub balance: f32,

    /// Mid (L+R)/2 RMS level
    pub mid_level: f32,

    /// Side (L-R)/2 RMS level
    pub side_level: f32,

    /// Left channel RMS level
    pub left_level: f32,

    /// Right channel RMS level
    pub right_level: f32,
}

impl StereoAnalysis {
    /// Check if the stereo signal has phase issues
    ///
    /// Returns true if correlation is significantly negative,
    /// which can cause problems when summed to mono.
    pub fn has_phase_issues(&self) -> bool {
        self.correlation < -0.3
    }

    /// Check if the signal is essentially mono
    pub fn is_mono(&self) -> bool {
        self.correlation > 0.95
    }

    /// Get M/S ratio in dB
    ///
    /// Positive values = more mid (narrower)
    /// Negative values = more side (wider)
    pub fn ms_ratio_db(&self) -> f32 {
        if self.side_level > 0.0 && self.mid_level > 0.0 {
            20.0 * (self.mid_level / self.side_level).log10()
        } else if self.mid_level > 0.0 {
            f32::INFINITY // Pure mid
        } else if self.side_level > 0.0 {
            f32::NEG_INFINITY // Pure side
        } else {
            0.0 // Silence
        }
    }
}

/// Real-time stereo correlation meter
///
/// Maintains smoothed state for continuous monitoring.
pub struct CorrelationMeter {
    sample_rate: f64,
    /// Smoothing coefficient (0.0 = no smoothing, 1.0 = infinite smoothing)
    smoothing: f32,
    /// Current smoothed analysis
    current: StereoAnalysis,
    /// Attack time in seconds (for increasing values)
    attack_time: f32,
    /// Release time in seconds (for decreasing values)
    release_time: f32,
}

impl CorrelationMeter {
    /// Create a new correlation meter
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate,
            smoothing: 0.9,
            current: StereoAnalysis::default(),
            attack_time: 0.01, // 10ms attack
            release_time: 0.1, // 100ms release
        }
    }

    /// Set smoothing coefficient (0.0 - 0.99)
    pub fn set_smoothing(&mut self, smoothing: f32) {
        self.smoothing = smoothing.clamp(0.0, 0.99);
    }

    /// Set attack/release times in milliseconds
    pub fn set_times(&mut self, attack_ms: f32, release_ms: f32) {
        self.attack_time = attack_ms / 1000.0;
        self.release_time = release_ms / 1000.0;
    }

    /// Process a stereo buffer and update internal state
    ///
    /// # Arguments
    /// * `left` - Left channel samples
    /// * `right` - Right channel samples
    ///
    /// # Returns
    /// Instantaneous (non-smoothed) analysis of this buffer
    pub fn process(&mut self, left: &[f32], right: &[f32]) -> StereoAnalysis {
        let instant = analyze_stereo(left, right);

        // Calculate smoothing coefficients based on attack/release
        let buffer_duration = left.len() as f32 / self.sample_rate as f32;

        // Apply asymmetric smoothing (faster attack, slower release)
        self.current.correlation = self.smooth_value(
            self.current.correlation,
            instant.correlation,
            buffer_duration,
        );
        self.current.width = self.smooth_value(self.current.width, instant.width, buffer_duration);
        self.current.balance =
            self.smooth_value(self.current.balance, instant.balance, buffer_duration);
        self.current.mid_level =
            self.smooth_value(self.current.mid_level, instant.mid_level, buffer_duration);
        self.current.side_level =
            self.smooth_value(self.current.side_level, instant.side_level, buffer_duration);
        self.current.left_level =
            self.smooth_value(self.current.left_level, instant.left_level, buffer_duration);
        self.current.right_level = self.smooth_value(
            self.current.right_level,
            instant.right_level,
            buffer_duration,
        );

        instant
    }

    /// Get current smoothed analysis
    pub fn current(&self) -> StereoAnalysis {
        self.current
    }

    /// Reset the meter state
    pub fn reset(&mut self) {
        self.current = StereoAnalysis::default();
    }

    fn smooth_value(&self, current: f32, target: f32, buffer_duration: f32) -> f32 {
        let time_constant = if target > current {
            self.attack_time
        } else {
            self.release_time
        };

        if time_constant <= 0.0 {
            return target;
        }

        let coeff = (-buffer_duration / time_constant).exp();
        current * coeff + target * (1.0 - coeff)
    }
}

/// Analyze a stereo buffer (non-streaming, instant analysis)
///
/// # Arguments
/// * `left` - Left channel samples
/// * `right` - Right channel samples
///
/// # Returns
/// Complete stereo analysis
pub fn analyze_stereo(left: &[f32], right: &[f32]) -> StereoAnalysis {
    let len = left.len().min(right.len());
    if len == 0 {
        return StereoAnalysis::default();
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

    // RMS levels
    let left_rms = (sum_l_sq / n).sqrt() as f32;
    let right_rms = (sum_r_sq / n).sqrt() as f32;
    let mid_rms = (sum_mid_sq / n).sqrt() as f32;
    let side_rms = (sum_side_sq / n).sqrt() as f32;

    // Correlation coefficient
    // r = Σ(L*R) / sqrt(Σ(L²) * Σ(R²))
    let correlation = if sum_l_sq > 0.0 && sum_r_sq > 0.0 {
        (sum_lr / (sum_l_sq.sqrt() * sum_r_sq.sqrt())) as f32
    } else {
        0.0
    };

    // Stereo width (derived from correlation)
    // width = 1 - correlation maps to: mono=0, normal=0.5-1, wide=1-2
    let width = 1.0 - correlation;

    // Balance
    let total_level = left_rms + right_rms;
    let balance = if total_level > 0.0 {
        (right_rms - left_rms) / total_level
    } else {
        0.0
    };

    StereoAnalysis {
        correlation,
        width,
        balance,
        mid_level: mid_rms,
        side_level: side_rms,
        left_level: left_rms,
        right_level: right_rms,
    }
}

/// Analyze stereo from interleaved samples
pub fn analyze_stereo_interleaved(samples: &[f32]) -> StereoAnalysis {
    if samples.len() < 2 {
        return StereoAnalysis::default();
    }

    let len = samples.len() / 2;
    let mut left = Vec::with_capacity(len);
    let mut right = Vec::with_capacity(len);

    for i in 0..len {
        left.push(samples[i * 2]);
        right.push(samples[i * 2 + 1]);
    }

    analyze_stereo(&left, &right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mono_signal() {
        // Identical L/R = perfect correlation
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();

        let analysis = analyze_stereo(&samples, &samples);

        assert!(
            analysis.correlation > 0.99,
            "Mono signal should have ~1.0 correlation"
        );
        assert!(analysis.is_mono());
        assert!(!analysis.has_phase_issues());
        assert!(analysis.width < 0.1, "Mono signal should have ~0 width");
        assert!(
            analysis.balance.abs() < 0.01,
            "Identical channels should be balanced"
        );
        assert!(
            analysis.side_level < 0.001,
            "Mono signal should have ~0 side level"
        );
    }

    #[test]
    fn test_out_of_phase() {
        // L and -R = out of phase
        let left: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let right: Vec<f32> = left.iter().map(|&s| -s).collect();

        let analysis = analyze_stereo(&left, &right);

        assert!(
            analysis.correlation < -0.99,
            "Out of phase should have ~-1.0 correlation"
        );
        assert!(analysis.has_phase_issues());
        assert!(analysis.width > 1.9, "Out of phase should have ~2.0 width");
        assert!(
            analysis.mid_level < 0.001,
            "Out of phase should have ~0 mid level"
        );
    }

    #[test]
    fn test_uncorrelated() {
        // Different frequencies should be less correlated
        let left: Vec<f32> = (0..10000).map(|i| (i as f32 * 0.1).sin()).collect();
        let right: Vec<f32> = (0..10000).map(|i| (i as f32 * 0.17).sin()).collect(); // Different frequency

        let analysis = analyze_stereo(&left, &right);

        // Different frequencies should have lower correlation than identical signals
        // but may not be exactly 0 due to harmonic relationships
        assert!(
            analysis.correlation < 0.9,
            "Different frequency signals should have lower correlation, got {}",
            analysis.correlation
        );
    }

    #[test]
    fn test_balance() {
        // Left only
        let left: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let right: Vec<f32> = vec![0.0; 1000];

        let analysis = analyze_stereo(&left, &right);
        assert!(
            analysis.balance < -0.9,
            "Left-only should have negative balance"
        );

        // Right only
        let analysis = analyze_stereo(&right, &left);
        assert!(
            analysis.balance > 0.9,
            "Right-only should have positive balance"
        );
    }

    #[test]
    fn test_silence() {
        let silence = vec![0.0f32; 1000];
        let analysis = analyze_stereo(&silence, &silence);

        assert_eq!(analysis.left_level, 0.0);
        assert_eq!(analysis.right_level, 0.0);
        assert_eq!(analysis.correlation, 0.0);
    }

    #[test]
    fn test_correlation_meter() {
        let sample_rate = 44100.0;
        let mut meter = CorrelationMeter::new(sample_rate);

        let left: Vec<f32> = (0..1024).map(|i| (i as f32 / 100.0).sin()).collect();
        let right = left.clone();

        // Process several buffers
        for _ in 0..10 {
            meter.process(&left, &right);
        }

        let current = meter.current();
        assert!(
            current.correlation > 0.9,
            "Should converge to ~1.0 for mono signal"
        );
    }

    #[test]
    fn test_ms_ratio() {
        // Mono signal: all mid, no side
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin()).collect();
        let analysis = analyze_stereo(&samples, &samples);

        assert!(
            analysis.ms_ratio_db() > 40.0,
            "Mono should have high M/S ratio"
        );

        // Out of phase: all side, no mid
        let right: Vec<f32> = samples.iter().map(|&s| -s).collect();
        let analysis = analyze_stereo(&samples, &right);

        assert!(
            analysis.ms_ratio_db() < -40.0,
            "Out of phase should have low M/S ratio"
        );
    }

    #[test]
    fn test_interleaved() {
        let left: Vec<f32> = (0..100).map(|i| (i as f32 / 10.0).sin()).collect();
        let right: Vec<f32> = (0..100).map(|i| (i as f32 / 10.0).cos()).collect();

        // Create interleaved
        let mut interleaved = Vec::with_capacity(200);
        for i in 0..100 {
            interleaved.push(left[i]);
            interleaved.push(right[i]);
        }

        let analysis1 = analyze_stereo(&left, &right);
        let analysis2 = analyze_stereo_interleaved(&interleaved);

        assert!((analysis1.correlation - analysis2.correlation).abs() < 0.001);
        assert!((analysis1.left_level - analysis2.left_level).abs() < 0.001);
    }
}
