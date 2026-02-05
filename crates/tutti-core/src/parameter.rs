//! Parameter range and scaling for automation and plugin parameters.
//!
//! Provides normalized (0.0-1.0) ↔ real value conversion with different scaling algorithms.
//!
//! # Example
//!
//! ```
//! use tutti_core::{ParameterRange, ParameterScale};
//!
//! // Filter cutoff: 20Hz to 20kHz, logarithmic scaling
//! let cutoff = ParameterRange::new(20.0, 20000.0, 1000.0, ParameterScale::Logarithmic);
//!
//! // Automation stores normalized 0.0-1.0
//! let normalized = 0.5;
//! let freq_hz = cutoff.denormalize(normalized);  // ~632 Hz (geometric mean)
//!
//! // Convert back to normalized
//! let back = cutoff.normalize(freq_hz);  // ~0.5
//! ```


/// How a parameter value is scaled between normalized (0-1) and real values.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ParameterScale {
    /// Linear mapping (default)
    ///
    /// `real = min + normalized * (max - min)`
    #[default]
    Linear,

    /// Logarithmic scaling (for frequencies, gains)
    ///
    /// `real = min * (max/min)^normalized`
    ///
    /// Requires `min > 0` and `max > min`.
    Logarithmic,

    /// Exponential curve with configurable shape
    ///
    /// `curve > 1.0`: More resolution at low end
    /// `curve < 1.0`: More resolution at high end
    /// `curve = 1.0`: Linear (equivalent to Linear)
    Exponential {
        /// Curve shape factor (typically 2.0-4.0)
        curve: f32,
    },

    /// On/off toggle (normalized < 0.5 = off, >= 0.5 = on)
    ///
    /// Denormalizes to `min` (off) or `max` (on).
    Toggle,

    /// Discrete integer steps
    ///
    /// Values are quantized to integers between `min` and `max`.
    Integer,
}

/// Parameter range with scaling for automation and plugin parameters.
///
/// Stores the valid range and default value, and provides conversion between
/// normalized (0.0-1.0) and real parameter values.
#[derive(Debug, Clone)]
pub struct ParameterRange {
    /// Minimum real value
    pub min: f32,
    /// Maximum real value
    pub max: f32,
    /// Default real value
    pub default: f32,
    /// Scaling algorithm
    pub scale: ParameterScale,
}

impl ParameterRange {
    /// Create a new parameter range.
    ///
    /// # Arguments
    ///
    /// * `min` - Minimum real value
    /// * `max` - Maximum real value (must be > min)
    /// * `default` - Default real value (will be clamped to range)
    /// * `scale` - Scaling algorithm
    pub fn new(min: f32, max: f32, default: f32, scale: ParameterScale) -> Self {
        debug_assert!(max > min, "max must be greater than min");

        Self {
            min,
            max,
            default: default.clamp(min, max),
            scale,
        }
    }

    /// Create a linear parameter range.
    pub fn linear(min: f32, max: f32, default: f32) -> Self {
        Self::new(min, max, default, ParameterScale::Linear)
    }

    /// Create a logarithmic parameter range (for frequencies, gains).
    ///
    /// # Panics
    ///
    /// Panics in debug mode if `min <= 0`.
    pub fn logarithmic(min: f32, max: f32, default: f32) -> Self {
        debug_assert!(min > 0.0, "logarithmic scale requires min > 0");
        Self::new(min, max, default, ParameterScale::Logarithmic)
    }

    /// Create an exponential parameter range.
    ///
    /// # Arguments
    ///
    /// * `curve` - Shape factor. Values > 1.0 give more resolution at low end.
    pub fn exponential(min: f32, max: f32, default: f32, curve: f32) -> Self {
        Self::new(min, max, default, ParameterScale::Exponential { curve })
    }

    /// Create a toggle (on/off) parameter.
    ///
    /// * `min` is the "off" value
    /// * `max` is the "on" value
    /// * `default` should be either `min` or `max`
    pub fn toggle(off_value: f32, on_value: f32, default_on: bool) -> Self {
        Self::new(
            off_value,
            on_value,
            if default_on { on_value } else { off_value },
            ParameterScale::Toggle,
        )
    }

    /// Create an integer parameter range.
    pub fn integer(min: i32, max: i32, default: i32) -> Self {
        Self::new(
            min as f32,
            max as f32,
            default as f32,
            ParameterScale::Integer,
        )
    }

    /// Convert a real value to normalized (0.0-1.0).
    #[inline]
    pub fn normalize(&self, value: f32) -> f32 {
        let value = value.clamp(self.min, self.max);
        let range = self.max - self.min;

        if range <= 0.0 {
            return 0.0;
        }

        match self.scale {
            ParameterScale::Linear => (value - self.min) / range,

            ParameterScale::Logarithmic => {
                if self.min <= 0.0 {
                    // Fallback to linear if min is invalid
                    (value - self.min) / range
                } else {
                    let log_min = self.min.ln();
                    let log_max = self.max.ln();
                    (value.ln() - log_min) / (log_max - log_min)
                }
            }

            ParameterScale::Exponential { curve } => {
                let linear = (value - self.min) / range;
                if curve <= 0.0 || curve == 1.0 {
                    linear
                } else {
                    linear.powf(1.0 / curve)
                }
            }

            ParameterScale::Toggle => {
                if value >= (self.min + self.max) / 2.0 {
                    1.0
                } else {
                    0.0
                }
            }

            ParameterScale::Integer => {
                let int_value = value.round();
                (int_value - self.min) / range
            }
        }
    }

    /// Convert a normalized value (0.0-1.0) to a real value.
    #[inline]
    pub fn denormalize(&self, normalized: f32) -> f32 {
        let normalized = normalized.clamp(0.0, 1.0);
        let range = self.max - self.min;

        match self.scale {
            ParameterScale::Linear => self.min + normalized * range,

            ParameterScale::Logarithmic => {
                if self.min <= 0.0 {
                    // Fallback to linear if min is invalid
                    self.min + normalized * range
                } else {
                    let log_min = self.min.ln();
                    let log_max = self.max.ln();
                    (log_min + normalized * (log_max - log_min)).exp()
                }
            }

            ParameterScale::Exponential { curve } => {
                let shaped = if curve <= 0.0 || curve == 1.0 {
                    normalized
                } else {
                    normalized.powf(curve)
                };
                self.min + shaped * range
            }

            ParameterScale::Toggle => {
                if normalized >= 0.5 {
                    self.max
                } else {
                    self.min
                }
            }

            ParameterScale::Integer => {
                let continuous = self.min + normalized * range;
                continuous.round()
            }
        }
    }

    /// Clamp a real value to this parameter's range.
    #[inline]
    pub fn clamp(&self, value: f32) -> f32 {
        value.clamp(self.min, self.max)
    }

    /// Get the normalized value of the default.
    #[inline]
    pub fn default_normalized(&self) -> f32 {
        self.normalize(self.default)
    }

    /// Check if a real value is within range.
    #[inline]
    pub fn contains(&self, value: f32) -> bool {
        value >= self.min && value <= self.max
    }

    /// Get the range span (max - min).
    #[inline]
    pub fn span(&self) -> f32 {
        self.max - self.min
    }

    /// Convert dB value to linear amplitude.
    ///
    /// Useful for gain parameters stored in dB.
    #[inline]
    pub fn db_to_linear(db: f32) -> f32 {
        10.0_f32.powf(db / 20.0)
    }

    /// Convert linear amplitude to dB.
    ///
    /// Returns -inf for amplitude <= 0.
    #[inline]
    pub fn linear_to_db(linear: f32) -> f32 {
        if linear <= 0.0 {
            f32::NEG_INFINITY
        } else {
            20.0 * linear.log10()
        }
    }
}

impl Default for ParameterRange {
    fn default() -> Self {
        Self::linear(0.0, 1.0, 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Approximate equality for floats.
    /// Uses relative epsilon for large values, absolute epsilon for small values.
    fn approx_eq(a: f32, b: f32) -> bool {
        let abs_diff = (a - b).abs();
        let max_val = a.abs().max(b.abs());

        if max_val < 1.0 {
            // For small values, use absolute epsilon
            abs_diff < 0.0001
        } else {
            // For larger values, use relative epsilon (0.001% tolerance)
            abs_diff / max_val < 0.00001
        }
    }

    #[test]
    fn test_linear_normalize_denormalize() {
        let range = ParameterRange::linear(0.0, 100.0, 50.0);

        assert!(approx_eq(range.normalize(0.0), 0.0));
        assert!(approx_eq(range.normalize(50.0), 0.5));
        assert!(approx_eq(range.normalize(100.0), 1.0));

        assert!(approx_eq(range.denormalize(0.0), 0.0));
        assert!(approx_eq(range.denormalize(0.5), 50.0));
        assert!(approx_eq(range.denormalize(1.0), 100.0));
    }

    #[test]
    fn test_linear_roundtrip() {
        let range = ParameterRange::linear(-10.0, 10.0, 0.0);

        for value in [-10.0, -5.0, 0.0, 5.0, 10.0] {
            let normalized = range.normalize(value);
            let back = range.denormalize(normalized);
            assert!(approx_eq(value, back), "Roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_logarithmic_normalize_denormalize() {
        let range = ParameterRange::logarithmic(20.0, 20000.0, 1000.0);

        // At 0.5 normalized, should be geometric mean: sqrt(20 * 20000) = 632.45...
        let mid = range.denormalize(0.5);
        let expected_mid = (20.0_f32 * 20000.0).sqrt();
        assert!(
            approx_eq(mid, expected_mid),
            "Expected ~{}, got {}",
            expected_mid,
            mid
        );

        // Endpoints
        assert!(approx_eq(range.denormalize(0.0), 20.0));
        assert!(approx_eq(range.denormalize(1.0), 20000.0));
    }

    #[test]
    fn test_logarithmic_roundtrip() {
        let range = ParameterRange::logarithmic(20.0, 20000.0, 1000.0);

        for value in [20.0, 100.0, 1000.0, 10000.0, 20000.0] {
            let normalized = range.normalize(value);
            let back = range.denormalize(normalized);
            assert!(
                (value - back).abs() / value < 0.001,
                "Roundtrip failed for {}: got {}",
                value,
                back
            );
        }
    }

    #[test]
    fn test_exponential_curve() {
        let range = ParameterRange::exponential(0.0, 1.0, 0.5, 2.0);

        // With curve=2.0, denormalize(0.5) = 0.5^2 = 0.25
        assert!(approx_eq(range.denormalize(0.5), 0.25));

        // Endpoints should be unchanged
        assert!(approx_eq(range.denormalize(0.0), 0.0));
        assert!(approx_eq(range.denormalize(1.0), 1.0));
    }

    #[test]
    fn test_toggle() {
        let range = ParameterRange::toggle(0.0, 1.0, false);

        assert!(approx_eq(range.denormalize(0.0), 0.0));
        assert!(approx_eq(range.denormalize(0.49), 0.0));
        assert!(approx_eq(range.denormalize(0.5), 1.0));
        assert!(approx_eq(range.denormalize(1.0), 1.0));

        assert!(approx_eq(range.normalize(0.0), 0.0));
        assert!(approx_eq(range.normalize(1.0), 1.0));
    }

    #[test]
    fn test_integer() {
        let range = ParameterRange::integer(0, 10, 5);

        // Should round to nearest integer
        assert!(approx_eq(range.denormalize(0.0), 0.0));
        assert!(approx_eq(range.denormalize(0.5), 5.0));
        assert!(approx_eq(range.denormalize(1.0), 10.0));

        // Test intermediate values round correctly
        assert!(approx_eq(range.denormalize(0.15), 2.0)); // 1.5 rounds to 2
        assert!(approx_eq(range.denormalize(0.35), 4.0)); // 3.5 rounds to 4
    }

    #[test]
    fn test_clamp() {
        let range = ParameterRange::linear(0.0, 100.0, 50.0);

        assert!(approx_eq(range.clamp(-10.0), 0.0));
        assert!(approx_eq(range.clamp(50.0), 50.0));
        assert!(approx_eq(range.clamp(110.0), 100.0));
    }

    #[test]
    fn test_default_normalized() {
        let range = ParameterRange::linear(0.0, 100.0, 25.0);
        assert!(approx_eq(range.default_normalized(), 0.25));
    }

    #[test]
    fn test_db_conversion() {
        // 0 dB = 1.0 linear
        assert!(approx_eq(ParameterRange::db_to_linear(0.0), 1.0));

        // -6 dB ≈ 0.5 linear
        let minus_6db = ParameterRange::db_to_linear(-6.0);
        assert!(
            (minus_6db - 0.5).abs() < 0.02,
            "Expected ~0.5, got {}",
            minus_6db
        );

        // +6 dB ≈ 2.0 linear
        let plus_6db = ParameterRange::db_to_linear(6.0);
        assert!(
            (plus_6db - 2.0).abs() < 0.05,
            "Expected ~2.0, got {}",
            plus_6db
        );

        // Roundtrip
        let db = -12.0;
        let linear = ParameterRange::db_to_linear(db);
        let back = ParameterRange::linear_to_db(linear);
        assert!(approx_eq(db, back), "dB roundtrip failed: {} -> {}", db, back);
    }

    #[test]
    fn test_contains() {
        let range = ParameterRange::linear(0.0, 100.0, 50.0);

        assert!(range.contains(0.0));
        assert!(range.contains(50.0));
        assert!(range.contains(100.0));
        assert!(!range.contains(-1.0));
        assert!(!range.contains(101.0));
    }
}
