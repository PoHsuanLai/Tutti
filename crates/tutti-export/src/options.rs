//! Export options.

use crate::dsp::ResampleQuality;

/// Audio format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFormat {
    #[default]
    Wav,
    Flac,
}

impl AudioFormat {
    /// File extension (without dot).
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Flac => "flac",
        }
    }
}

/// Bit depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

impl BitDepth {
    /// Bits per sample.
    pub fn bits(&self) -> u16 {
        match self {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        }
    }
}

/// Dithering algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DitherType {
    None,
    Rectangular,
    #[default]
    Triangular,
    NoiseShaped,
}

/// Normalization mode.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum NormalizationMode {
    #[default]
    None,
    /// Peak normalization (dB).
    Peak(f64),
    /// Loudness normalization (EBU R128).
    Loudness {
        target_lufs: f64,
        true_peak_dbtp: f64,
    },
}

impl NormalizationMode {
    /// Loudness normalization with default true peak limit (-1.0 dBTP).
    pub fn lufs(target_lufs: f64) -> Self {
        Self::Loudness {
            target_lufs,
            true_peak_dbtp: -1.0,
        }
    }
}

/// FLAC encoding options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlacOptions {
    /// Compression level (0-8).
    pub compression_level: u8,
}

impl Default for FlacOptions {
    fn default() -> Self {
        Self {
            compression_level: 5,
        }
    }
}

/// Export options.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    /// Audio format.
    pub format: AudioFormat,
    /// Bit depth.
    pub bit_depth: BitDepth,
    /// Target sample rate (None = keep original).
    pub sample_rate: Option<u32>,
    /// Source sample rate.
    pub source_sample_rate: u32,
    /// Normalization mode.
    pub normalization: NormalizationMode,
    /// Dithering type.
    pub dither: DitherType,
    /// Resampling quality.
    pub resample_quality: ResampleQuality,
    /// Export as mono.
    pub mono: bool,
    /// FLAC options.
    pub flac: FlacOptions,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: AudioFormat::Wav,
            bit_depth: BitDepth::Int24,
            sample_rate: None,
            source_sample_rate: 44100,
            normalization: NormalizationMode::None,
            dither: DitherType::Triangular,
            resample_quality: ResampleQuality::Medium,
            mono: false,
            flac: FlacOptions::default(),
        }
    }
}

impl ExportOptions {
    /// Effective output sample rate.
    pub fn output_sample_rate(&self) -> u32 {
        self.sample_rate.unwrap_or(self.source_sample_rate)
    }

    /// Whether resampling is needed.
    pub fn needs_resampling(&self) -> bool {
        self.sample_rate
            .map(|r| r != self.source_sample_rate)
            .unwrap_or(false)
    }
}
