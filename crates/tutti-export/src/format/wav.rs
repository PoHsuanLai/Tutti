//! WAV format encoder using hound
//!
//! Supports 16-bit, 24-bit, and 32-bit float WAV files.

use crate::error::{ExportError, Result};
use crate::options::{BitDepth, ExportOptions};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::{Seek, Write};
use std::path::Path;

/// WAV encoder configuration
#[derive(Debug, Clone)]
pub struct WavConfig {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Bit depth
    pub bit_depth: BitDepth,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
}

impl Default for WavConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            bit_depth: BitDepth::Int16,
            channels: 2,
        }
    }
}

impl WavConfig {
    /// Create a new WAV config for stereo output
    pub fn stereo(sample_rate: u32, bit_depth: BitDepth) -> Self {
        Self {
            sample_rate,
            bit_depth,
            channels: 2,
        }
    }

    /// Create a new WAV config for mono output
    pub fn mono(sample_rate: u32, bit_depth: BitDepth) -> Self {
        Self {
            sample_rate,
            bit_depth,
            channels: 1,
        }
    }
}

/// Export stereo audio to WAV file using ExportOptions
pub fn export_wav(path: &str, left: &[f32], right: &[f32], options: &ExportOptions) -> Result<()> {
    use crate::dsp::{
        apply_dither, calculate_loudness, normalize_loudness, normalize_peak, resample_stereo,
        DitherState,
    };
    use crate::options::NormalizationMode;

    let config = WavConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
    };

    // Copy to mutable buffers for processing
    let mut left_proc = left.to_vec();
    let mut right_proc = right.to_vec();

    // Apply resampling if needed
    if let Some(target_rate) = options.sample_rate.rate() {
        let (left_resampled, right_resampled) = resample_stereo(
            &left_proc,
            &right_proc,
            options.source_sample_rate,
            target_rate,
            options.resample_quality,
        )?;
        left_proc = left_resampled;
        right_proc = right_resampled;
    }

    // Apply normalization
    match options.normalization {
        NormalizationMode::None => {}
        NormalizationMode::Peak(target_db) => {
            normalize_peak(&mut left_proc, &mut right_proc, target_db);
        }
        NormalizationMode::Loudness {
            target_lufs,
            true_peak_dbtp,
        } => {
            let current = calculate_loudness(&left_proc, &right_proc, config.sample_rate);
            normalize_loudness(
                &mut left_proc,
                &mut right_proc,
                current.integrated_lufs,
                target_lufs,
                true_peak_dbtp,
            );
        }
    }

    // Apply dithering if needed
    if options.dither != crate::options::DitherType::None {
        let mut state = DitherState::new(options.dither);
        apply_dither(
            &mut left_proc,
            &mut right_proc,
            options.bit_depth.bits(),
            &mut state,
        );
    }

    if options.mono {
        // Downmix to mono
        let mono: Vec<f32> = left_proc
            .iter()
            .zip(right_proc.iter())
            .map(|(l, r)| (l + r) * 0.5)
            .collect();
        encode_wav_mono_file(&mono, Path::new(path), &config)
    } else {
        encode_wav_file(&left_proc, &right_proc, Path::new(path), &config)
    }
}

/// Encode stereo audio to WAV file
///
/// # Arguments
/// * `left` - Left channel samples (normalized -1.0 to 1.0)
/// * `right` - Right channel samples (normalized -1.0 to 1.0)
/// * `path` - Output file path
/// * `config` - WAV configuration
pub fn encode_wav_file(left: &[f32], right: &[f32], path: &Path, config: &WavConfig) -> Result<()> {
    if left.len() != right.len() {
        return Err(ExportError::InvalidData(
            "Left and right channels have different lengths".into(),
        ));
    }

    let spec = create_wav_spec(config);
    let mut writer =
        WavWriter::create(path, spec).map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

    write_samples(&mut writer, left, right, config)?;

    writer
        .finalize()
        .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

    Ok(())
}

/// Encode stereo audio to WAV in memory
///
/// # Arguments
/// * `left` - Left channel samples (normalized -1.0 to 1.0)
/// * `right` - Right channel samples (normalized -1.0 to 1.0)
/// * `config` - WAV configuration
///
/// # Returns
/// WAV file bytes
pub fn encode_wav_memory(left: &[f32], right: &[f32], config: &WavConfig) -> Result<Vec<u8>> {
    if left.len() != right.len() {
        return Err(ExportError::InvalidData(
            "Left and right channels have different lengths".into(),
        ));
    }

    let spec = create_wav_spec(config);
    let mut buffer = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buffer);
        let mut writer =
            WavWriter::new(cursor, spec).map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

        write_samples(&mut writer, left, right, config)?;

        // Finalize writes the header and flushes
        writer
            .finalize()
            .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
    }

    Ok(buffer)
}

/// Encode mono audio to WAV in memory
///
/// # Arguments
/// * `samples` - Mono audio samples (normalized -1.0 to 1.0)
/// * `config` - WAV configuration
///
/// # Returns
/// WAV file bytes
pub fn encode_wav_mono_memory(samples: &[f32], config: &WavConfig) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: config.sample_rate,
        bits_per_sample: config.bit_depth.bits(),
        sample_format: match config.bit_depth {
            BitDepth::Float32 => SampleFormat::Float,
            _ => SampleFormat::Int,
        },
    };

    let mut buffer = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buffer);
        let mut writer =
            WavWriter::new(cursor, spec).map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

        write_mono_samples(&mut writer, samples, config)?;

        writer
            .finalize()
            .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
    }

    Ok(buffer)
}

/// Encode mono audio to WAV file
pub fn encode_wav_mono_file(samples: &[f32], path: &Path, config: &WavConfig) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: config.sample_rate,
        bits_per_sample: config.bit_depth.bits(),
        sample_format: match config.bit_depth {
            BitDepth::Float32 => SampleFormat::Float,
            _ => SampleFormat::Int,
        },
    };

    let mut writer =
        WavWriter::create(path, spec).map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

    write_mono_samples(&mut writer, samples, config)?;

    writer
        .finalize()
        .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;

    Ok(())
}

/// Create hound WavSpec from our config
fn create_wav_spec(config: &WavConfig) -> WavSpec {
    let (bits_per_sample, sample_format) = match config.bit_depth {
        BitDepth::Int16 => (16, SampleFormat::Int),
        BitDepth::Int24 => (24, SampleFormat::Int),
        BitDepth::Float32 => (32, SampleFormat::Float),
    };

    WavSpec {
        channels: config.channels,
        sample_rate: config.sample_rate,
        bits_per_sample,
        sample_format,
    }
}

/// Write interleaved stereo samples to the writer
fn write_samples<W: Write + Seek>(
    writer: &mut WavWriter<W>,
    left: &[f32],
    right: &[f32],
    config: &WavConfig,
) -> Result<()> {
    match config.bit_depth {
        BitDepth::Int16 => {
            for i in 0..left.len() {
                let l = float_to_i16(left[i]);
                let r = float_to_i16(right[i]);
                writer
                    .write_sample(l)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
                writer
                    .write_sample(r)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
        BitDepth::Int24 => {
            for i in 0..left.len() {
                let l = float_to_i24(left[i]);
                let r = float_to_i24(right[i]);
                writer
                    .write_sample(l)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
                writer
                    .write_sample(r)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
        BitDepth::Float32 => {
            for i in 0..left.len() {
                writer
                    .write_sample(left[i])
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
                writer
                    .write_sample(right[i])
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
    }

    Ok(())
}

/// Write mono samples to the writer
fn write_mono_samples<W: Write + Seek>(
    writer: &mut WavWriter<W>,
    samples: &[f32],
    config: &WavConfig,
) -> Result<()> {
    match config.bit_depth {
        BitDepth::Int16 => {
            for &sample in samples {
                let s = float_to_i16(sample);
                writer
                    .write_sample(s)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
        BitDepth::Int24 => {
            for &sample in samples {
                let s = float_to_i24(sample);
                writer
                    .write_sample(s)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
        BitDepth::Float32 => {
            for &sample in samples {
                writer
                    .write_sample(sample)
                    .map_err(|e| ExportError::Io(std::io::Error::other(e)))?;
            }
        }
    }

    Ok(())
}

/// Convert float sample to 16-bit integer with clipping
#[inline]
fn float_to_i16(sample: f32) -> i16 {
    let clamped = sample.clamp(-1.0, 1.0);
    (clamped * 32767.0) as i16
}

/// Convert float sample to 24-bit integer (stored as i32) with clipping
#[inline]
fn float_to_i24(sample: f32) -> i32 {
    let clamped = sample.clamp(-1.0, 1.0);
    (clamped * 8388607.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wav_config_default() {
        let config = WavConfig::default();
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn test_wav_config_stereo() {
        let config = WavConfig::stereo(48000, BitDepth::Int24);
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn test_float_to_i16() {
        assert_eq!(float_to_i16(0.0), 0);
        assert_eq!(float_to_i16(1.0), 32767);
        assert_eq!(float_to_i16(-1.0), -32767);
        // Test clipping
        assert_eq!(float_to_i16(1.5), 32767);
        assert_eq!(float_to_i16(-1.5), -32767);
    }

    #[test]
    fn test_float_to_i24() {
        assert_eq!(float_to_i24(0.0), 0);
        assert_eq!(float_to_i24(1.0), 8388607);
        assert_eq!(float_to_i24(-1.0), -8388607);
    }

    #[test]
    fn test_encode_wav_memory() {
        let left = vec![0.0, 0.5, -0.5];
        let right = vec![0.1, -0.1, 0.0];
        let config = WavConfig::default();

        let bytes = encode_wav_memory(&left, &right, &config).unwrap();

        // Check WAV header magic
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");

        // Should be non-empty
        assert!(bytes.len() > 44); // Minimum WAV header size
    }

    #[test]
    fn test_encode_wav_mono_memory() {
        let samples = vec![0.0, 0.5, -0.5];
        let config = WavConfig::mono(44100, BitDepth::Int16);

        let bytes = encode_wav_mono_memory(&samples, &config).unwrap();

        // Check WAV header magic
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");

        // Should be non-empty
        assert!(bytes.len() > 44);
    }

    #[test]
    fn test_encode_wav_memory_mismatched_lengths() {
        let left = vec![0.0, 0.5];
        let right = vec![0.1];
        let config = WavConfig::default();

        let result = encode_wav_memory(&left, &right, &config);
        assert!(result.is_err());
    }
}
