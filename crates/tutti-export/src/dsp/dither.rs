//! Dithering algorithms for bit depth reduction
//!
//! Dithering adds small amounts of noise to reduce quantization distortion
//! when converting from higher to lower bit depths.

use crate::options::DitherType;

/// Dither state for processing
pub struct DitherState {
    /// Random state for noise generation
    random_state: u32,
    /// Previous error for noise shaping
    error_l: f32,
    error_r: f32,
    /// Dither type
    dither_type: DitherType,
}

impl DitherState {
    /// Create a new dither state
    pub fn new(dither_type: DitherType) -> Self {
        Self {
            random_state: 0x12345678,
            error_l: 0.0,
            error_r: 0.0,
            dither_type,
        }
    }

    /// Generate a random number using xorshift
    #[inline]
    fn random(&mut self) -> u32 {
        let mut x = self.random_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.random_state = x;
        x
    }

    /// Generate rectangular dither noise (-0.5 to 0.5 LSB)
    #[inline]
    fn rectangular_noise(&mut self) -> f32 {
        (self.random() as f32 / u32::MAX as f32) - 0.5
    }

    /// Generate triangular PDF dither noise (-1.0 to 1.0 LSB)
    #[inline]
    fn triangular_noise(&mut self) -> f32 {
        let r1 = self.random() as f32 / u32::MAX as f32;
        let r2 = self.random() as f32 / u32::MAX as f32;
        r1 - r2
    }
}

/// Apply dithering to a stereo audio buffer
///
/// # Arguments
/// * `left` - Left channel (modified in place)
/// * `right` - Right channel (modified in place)
/// * `target_bits` - Target bit depth (16 or 24)
/// * `state` - Dither state (maintains continuity between calls)
pub fn apply_dither(
    left: &mut [f32],
    right: &mut [f32],
    target_bits: u16,
    state: &mut DitherState,
) {
    if state.dither_type == DitherType::None {
        return;
    }

    // Calculate quantization step size
    let max_value = (1 << (target_bits - 1)) as f32;
    let lsb = 1.0 / max_value;

    match state.dither_type {
        DitherType::None => {}
        DitherType::Rectangular => {
            for i in 0..left.len() {
                left[i] += state.rectangular_noise() * lsb;
                right[i] += state.rectangular_noise() * lsb;
            }
        }
        DitherType::Triangular => {
            for i in 0..left.len() {
                left[i] += state.triangular_noise() * lsb;
                right[i] += state.triangular_noise() * lsb;
            }
        }
        DitherType::NoiseShaped => {
            // Simple first-order noise shaping
            for i in 0..left.len() {
                // Add dither noise
                let dither_l = state.triangular_noise() * lsb;
                let dither_r = state.triangular_noise() * lsb;

                // Apply noise shaping (feed back previous quantization error)
                let shaped_l = left[i] + dither_l - state.error_l * 0.5;
                let shaped_r = right[i] + dither_r - state.error_r * 0.5;

                // Quantize
                let quantized_l = (shaped_l * max_value).round() / max_value;
                let quantized_r = (shaped_r * max_value).round() / max_value;

                // Calculate error for next sample
                state.error_l = quantized_l - left[i];
                state.error_r = quantized_r - right[i];

                left[i] = quantized_l;
                right[i] = quantized_r;
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dither_state_creation() {
        let state = DitherState::new(DitherType::Triangular);
        assert_eq!(state.dither_type, DitherType::Triangular);
    }

    #[test]
    fn test_no_dither() {
        let mut left = vec![0.5, -0.5, 0.25];
        let mut right = vec![0.5, -0.5, 0.25];
        let original_left = left.clone();
        let original_right = right.clone();

        let mut state = DitherState::new(DitherType::None);
        apply_dither(&mut left, &mut right, 16, &mut state);

        assert_eq!(left, original_left);
        assert_eq!(right, original_right);
    }

    #[test]
    fn test_rectangular_dither() {
        let mut left = vec![0.0; 1000];
        let mut right = vec![0.0; 1000];

        let mut state = DitherState::new(DitherType::Rectangular);
        apply_dither(&mut left, &mut right, 16, &mut state);

        // Check that dither was applied (samples should be non-zero)
        let non_zero = left.iter().filter(|&&x| x != 0.0).count();
        assert!(non_zero > 900, "Expected most samples to have dither noise");

        // Check that noise is bounded
        let max_noise = 1.0 / 32768.0; // 16-bit LSB
        for &sample in &left {
            assert!(
                sample.abs() < max_noise * 2.0,
                "Noise exceeds expected bounds"
            );
        }
    }

    #[test]
    fn test_triangular_dither() {
        let mut left = vec![0.0; 1000];
        let mut right = vec![0.0; 1000];

        let mut state = DitherState::new(DitherType::Triangular);
        apply_dither(&mut left, &mut right, 16, &mut state);

        // TPDF dither should have larger range than rectangular
        let max_noise = 1.0 / 32768.0;
        let max_sample = left.iter().map(|x| x.abs()).fold(0.0f32, f32::max);

        // TPDF can go up to 2 LSBs
        assert!(max_sample < max_noise * 3.0);
    }

}
