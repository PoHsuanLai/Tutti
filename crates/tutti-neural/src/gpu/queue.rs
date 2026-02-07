//! Control parameter types for neural inference output.

/// Control parameters from neural inference (DDSP-style).
#[derive(Debug, Clone, Default)]
pub struct ControlParams {
    /// Fundamental frequencies (f0) per sample, typically 20-4000 Hz
    pub f0: Vec<f32>,
    /// Harmonic amplitudes per sample, 0.0-1.0
    pub amplitudes: Vec<f32>,
}
