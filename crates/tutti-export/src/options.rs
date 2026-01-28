//! Export options and configuration

/// Audio format for export
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AudioFormat {
    /// WAV format (uncompressed)
    #[default]
    Wav,
    /// FLAC format (lossless compression)
    Flac,
}

impl AudioFormat {
    /// Get the file extension for this format
    pub fn extension(&self) -> &'static str {
        match self {
            AudioFormat::Wav => "wav",
            AudioFormat::Flac => "flac",
        }
    }
}

/// Bit depth for audio export
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BitDepth {
    /// 16-bit integer (CD quality)
    Int16,
    /// 24-bit integer (professional audio)
    #[default]
    Int24,
    /// 32-bit float (maximum precision)
    Float32,
}

impl BitDepth {
    /// Get bits per sample
    pub fn bits(&self) -> u16 {
        match self {
            BitDepth::Int16 => 16,
            BitDepth::Int24 => 24,
            BitDepth::Float32 => 32,
        }
    }
}

/// Target sample rate for export
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SampleRateTarget {
    /// Keep original sample rate (no resampling)
    #[default]
    Original,
    /// CD quality
    Rate44100,
    /// Professional video
    Rate48000,
    /// High-resolution audio
    Rate88200,
    /// High-resolution audio
    Rate96000,
    /// Custom sample rate
    Custom(u32),
}

impl SampleRateTarget {
    /// Get the sample rate in Hz, or None for Original
    pub fn rate(&self) -> Option<u32> {
        match self {
            SampleRateTarget::Original => None,
            SampleRateTarget::Rate44100 => Some(44100),
            SampleRateTarget::Rate48000 => Some(48000),
            SampleRateTarget::Rate88200 => Some(88200),
            SampleRateTarget::Rate96000 => Some(96000),
            SampleRateTarget::Custom(rate) => Some(*rate),
        }
    }
}

/// Dithering algorithm for bit depth reduction
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DitherType {
    /// No dithering
    None,
    /// Simple rectangular dither (fastest)
    Rectangular,
    /// Triangular probability density function (best for most content)
    #[default]
    Triangular,
    /// Noise-shaped dither (most sophisticated, best perceptual quality)
    NoiseShaped,
}

/// Normalization mode for export
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum NormalizationMode {
    /// No normalization
    #[default]
    None,
    /// Peak normalization to specified dB level
    Peak(f64),
    /// Loudness normalization (EBU R128 / ITU-R BS.1770)
    Loudness {
        /// Target integrated loudness in LUFS
        target_lufs: f64,
        /// True peak ceiling in dBTP
        true_peak_dbtp: f64,
    },
}

/// Range of audio to export
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ExportRange {
    /// Export the entire project
    #[default]
    Full,
    /// Export a specific range (start sample, end sample)
    Range { start: usize, end: usize },
    /// Export loop region
    Loop,
    /// Export selected region
    Selection,
}

/// FLAC-specific encoding options
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FlacOptions {
    /// Compression level (0-8, higher = smaller file but slower)
    pub compression_level: u8,
}

impl Default for FlacOptions {
    fn default() -> Self {
        Self {
            compression_level: 5,
        }
    }
}

/// Complete export options
#[derive(Debug, Clone, PartialEq)]
pub struct ExportOptions {
    /// Audio format
    pub format: AudioFormat,
    /// Bit depth (for WAV/FLAC)
    pub bit_depth: BitDepth,
    /// Target sample rate
    pub sample_rate: SampleRateTarget,
    /// Source sample rate (used when sample_rate is Original)
    pub source_sample_rate: u32,
    /// Export range
    pub range: ExportRange,
    /// Normalization mode
    pub normalization: NormalizationMode,
    /// Dithering type
    pub dither: DitherType,
    /// Resampling quality (when sample_rate != Original)
    pub resample_quality: crate::dsp::ResampleQuality,
    /// Extra tail time in seconds (for reverb decay)
    pub tail_seconds: f64,
    /// Export as mono (downmix)
    pub mono: bool,
    /// FLAC-specific options
    pub flac: FlacOptions,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: AudioFormat::Wav,
            bit_depth: BitDepth::Int24,
            sample_rate: SampleRateTarget::Original,
            source_sample_rate: 44100,
            range: ExportRange::Full,
            normalization: NormalizationMode::None,
            dither: DitherType::Triangular,
            resample_quality: crate::dsp::ResampleQuality::Medium,
            tail_seconds: 0.0,
            mono: false,
            flac: FlacOptions::default(),
        }
    }
}

impl ExportOptions {
    /// Create new export options with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set audio format
    pub fn with_format(mut self, format: AudioFormat) -> Self {
        self.format = format;
        self
    }

    /// Set bit depth
    pub fn with_bit_depth(mut self, bit_depth: BitDepth) -> Self {
        self.bit_depth = bit_depth;
        self
    }

    /// Set target sample rate
    pub fn with_sample_rate(mut self, sample_rate: SampleRateTarget) -> Self {
        self.sample_rate = sample_rate;
        self
    }

    /// Set source sample rate
    pub fn with_source_sample_rate(mut self, rate: u32) -> Self {
        self.source_sample_rate = rate;
        self
    }

    /// Set normalization mode
    pub fn with_normalization(mut self, normalization: NormalizationMode) -> Self {
        self.normalization = normalization;
        self
    }

    /// Set dither type
    pub fn with_dither(mut self, dither: DitherType) -> Self {
        self.dither = dither;
        self
    }

    /// Set tail time in seconds
    pub fn with_tail(mut self, seconds: f64) -> Self {
        self.tail_seconds = seconds;
        self
    }

    /// Set mono export
    pub fn with_mono(mut self, mono: bool) -> Self {
        self.mono = mono;
        self
    }

    /// Get the effective output sample rate
    pub fn output_sample_rate(&self) -> u32 {
        self.sample_rate.rate().unwrap_or(self.source_sample_rate)
    }

    /// Check if resampling is needed
    pub fn needs_resampling(&self) -> bool {
        match self.sample_rate.rate() {
            Some(rate) => rate != self.source_sample_rate,
            None => false,
        }
    }
}
