//! Recording configuration types.

/// Recording source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingSource {
    MidiInput,
    AudioInput,
    InternalAudio,
    Pattern,
}

/// Recording mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingMode {
    Replace,
    Overdub,
    Loop,
}

/// Recording configuration.
#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub channel_index: usize,
    pub source: RecordingSource,
    pub mode: RecordingMode,
    pub quantize: Option<QuantizeSettings>,
    pub metronome: bool,
    pub preroll_beats: f64,
    pub punch_in: Option<f64>,
    pub punch_out: Option<f64>,
}

/// Quantize settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuantizeSettings {
    pub resolution: f64,
    pub strength: f32,
    pub swing: f32,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            channel_index: 0,
            source: RecordingSource::MidiInput,
            mode: RecordingMode::Replace,
            quantize: None,
            metronome: false,
            preroll_beats: 0.0,
            punch_in: None,
            punch_out: None,
        }
    }
}

impl QuantizeSettings {
    pub fn new(resolution: f64) -> Self {
        Self {
            resolution,
            strength: 1.0,
            swing: 0.0,
        }
    }

    pub fn quantize(&self, beat: f64) -> f64 {
        if self.strength == 0.0 {
            return beat;
        }

        let grid_position = (beat / self.resolution).round();
        let quantized = grid_position * self.resolution;

        let swung = if self.swing > 0.0 && grid_position % 2.0 == 1.0 {
            quantized + (self.resolution * self.swing as f64 * 0.5)
        } else {
            quantized
        };

        beat + (swung - beat) * self.strength as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantize_settings() {
        let settings = QuantizeSettings::new(0.25); // 16th notes

        // Test full quantization
        assert!((settings.quantize(0.13) - 0.25).abs() < 0.001);
        assert!((settings.quantize(0.87) - 0.75).abs() < 0.001);

        // Test partial quantization
        let partial = QuantizeSettings {
            resolution: 0.25,
            strength: 0.5,
            swing: 0.0,
        };
        let result = partial.quantize(0.13);
        assert!(result > 0.13 && result < 0.25); // Between original and quantized
    }

    #[test]
    fn test_swing() {
        let settings = QuantizeSettings {
            resolution: 0.25,
            strength: 1.0,
            swing: 0.5,
        };

        // First beat: no swing
        assert!((settings.quantize(0.0) - 0.0).abs() < 0.001);

        // Second beat: swung
        let swung = settings.quantize(0.25);
        assert!(swung > 0.25); // Shifted forward
    }
}
