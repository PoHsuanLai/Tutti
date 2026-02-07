//! Shared utilities for dynamics processors

/// Convert linear amplitude to decibels
#[inline]
pub(crate) fn amplitude_to_db(amp: f32) -> f32 {
    if amp <= 0.0 {
        -96.0 // Floor
    } else {
        20.0 * amp.log10()
    }
}

/// Convert decibels to linear amplitude
#[inline]
pub(crate) fn db_to_amplitude(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Calculate smoothing coefficient from time constant
#[inline]
pub(crate) fn time_to_coeff(time_seconds: f32, sample_rate: f64) -> f32 {
    if time_seconds <= 0.0 {
        1.0
    } else {
        (-1.0 / (time_seconds * sample_rate as f32)).exp()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_amplitude_db_conversion() {
        assert!((amplitude_to_db(1.0) - 0.0).abs() < 0.001);
        assert!((amplitude_to_db(0.5) - (-6.02)).abs() < 0.1);
        assert!((db_to_amplitude(0.0) - 1.0).abs() < 0.001);
        assert!((db_to_amplitude(-6.0) - 0.501).abs() < 0.01);
    }
}
