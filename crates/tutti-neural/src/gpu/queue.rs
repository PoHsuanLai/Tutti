//! Control parameter types for neural inference output.

/// Control parameters from neural inference.
///
/// Output of DDSP neural inference, used by the synthesizer
/// to generate audio in real-time. Delivered via crossbeam_channel.
#[derive(Debug, Clone, Default)]
pub struct ControlParams {
    /// Fundamental frequencies (f0) per sample
    ///
    /// Length: buffer_size (e.g., 512 samples)
    /// Range: 20 Hz - 4000 Hz (typical vocal range)
    pub f0: Vec<f32>,

    /// Harmonic amplitudes per sample
    ///
    /// Length: buffer_size (e.g., 512 samples)
    /// Range: 0.0 - 1.0 (normalized amplitude)
    pub amplitudes: Vec<f32>,
}
