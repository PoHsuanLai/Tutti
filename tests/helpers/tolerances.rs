//! Tolerance constants for audio testing.
//!
//! Different operations require different precision levels.
//! These constants are tuned based on FunDSP's test patterns and
//! common audio testing practices from Ardour and Zrythm.

/// Floating point rounding errors (for passthrough, exact gain).
/// Use for operations that should be mathematically exact.
pub const FLOAT_EPSILON: f32 = 1e-6;

/// DSP processing tolerance (filters, oscillators may have slight variations).
/// Accounts for SIMD vs scalar differences and algorithmic variations.
/// Based on FunDSP's tick vs process comparison tolerance.
pub const DSP_EPSILON: f32 = 1e-4;

/// Audio perceptual tolerance (~-60dB, inaudible differences).
/// Use for tests where perceptual equivalence matters more than exact values.
pub const PERCEPTUAL_EPSILON: f32 = 0.001;

/// Silence threshold (~-80dB).
/// Values below this are considered silent.
pub const SILENCE_THRESHOLD: f32 = 0.0001;

/// 16-bit quantization step size.
/// Use when testing bit-depth conversion to 16-bit.
pub const INT16_EPSILON: f32 = 1.0 / 32768.0;

/// 24-bit quantization step size.
/// Use when testing bit-depth conversion to 24-bit.
pub const INT24_EPSILON: f32 = 1.0 / 8388608.0;
