use crate::dsp::ResampleQuality;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFormat {
    #[default]
    Wav,
    Flac,
}

impl AudioFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Flac => "flac",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

impl BitDepth {
    pub fn bits(&self) -> u16 {
        match self {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DitherType {
    None,
    Rectangular,
    #[default]
    Triangular,
    NoiseShaped,
}

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
    pub const fn lufs(target_lufs: f64) -> Self {
        Self::Loudness {
            target_lufs,
            true_peak_dbtp: -1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlacOptions {
    pub compression_level: u8,
}

impl Default for FlacOptions {
    fn default() -> Self {
        Self {
            compression_level: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    pub format: AudioFormat,
    pub bit_depth: BitDepth,
    /// Target sample rate (None = keep original).
    pub sample_rate: Option<u32>,
    pub source_sample_rate: u32,
    pub normalization: NormalizationMode,
    pub dither: DitherType,
    pub resample_quality: ResampleQuality,
    pub mono: bool,
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
    pub fn output_sample_rate(&self) -> u32 {
        self.sample_rate.unwrap_or(self.source_sample_rate)
    }
}
