//! FLAC format encoder using flacenc
//!
//! Supports 16-bit and 24-bit lossless audio encoding.

use crate::error::{ExportError, Result};
use crate::options::{BitDepth, ExportOptions};
use flacenc::bitsink::ByteSink;
use flacenc::component::BitRepr;
use flacenc::config::Encoder as EncoderConfig;
use flacenc::encode_with_fixed_block_size;
use flacenc::error::Verify;
use flacenc::source::MemSource;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// FLAC encoder configuration
#[derive(Debug, Clone)]
pub struct FlacConfig {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Bit depth (16 or 24, 32-bit float not supported)
    pub bit_depth: BitDepth,
    /// Number of channels (1 = mono, 2 = stereo)
    pub channels: u16,
    /// Compression level (0-8, higher = smaller file, slower encoding)
    pub compression_level: u8,
    /// Block size (samples per block, affects compression efficiency)
    pub block_size: u32,
}

impl Default for FlacConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            bit_depth: BitDepth::Int16,
            channels: 2,
            compression_level: 5,
            block_size: 4096,
        }
    }
}

impl FlacConfig {
    /// Create a new FLAC config for stereo output
    pub fn stereo(sample_rate: u32, bit_depth: BitDepth) -> Self {
        Self {
            sample_rate,
            bit_depth,
            channels: 2,
            ..Default::default()
        }
    }

    /// Create a new FLAC config for mono output
    pub fn mono(sample_rate: u32, bit_depth: BitDepth) -> Self {
        Self {
            sample_rate,
            bit_depth,
            channels: 1,
            ..Default::default()
        }
    }

    /// Set compression level (0-8)
    pub fn with_compression(mut self, level: u8) -> Self {
        self.compression_level = level.min(8);
        self
    }

    /// Set block size
    pub fn with_block_size(mut self, size: u32) -> Self {
        self.block_size = size;
        self
    }
}

/// Export stereo audio to FLAC file using ExportOptions
pub fn export_flac(path: &str, left: &[f32], right: &[f32], options: &ExportOptions) -> Result<()> {
    use crate::dsp::{
        apply_dither, calculate_loudness, normalize_loudness, normalize_peak, DitherState,
    };
    use crate::options::NormalizationMode;

    let config = FlacConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
        compression_level: options.flac.compression_level,
        ..Default::default()
    };

    // Copy to mutable buffers for processing
    let mut left_proc = left.to_vec();
    let mut right_proc = right.to_vec();

    // Apply resampling if needed
    if let Some(target_rate) = options.sample_rate.rate() {
        let (left_resampled, right_resampled) = crate::dsp::resample_stereo(
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
        encode_flac_mono_file(&mono, Path::new(path), &config)
    } else {
        encode_flac_file(&left_proc, &right_proc, Path::new(path), &config)
    }
}

/// Encode stereo audio to FLAC file
pub fn encode_flac_file(
    left: &[f32],
    right: &[f32],
    path: &Path,
    config: &FlacConfig,
) -> Result<()> {
    let flac_data = encode_flac_memory(left, right, config)?;

    let mut file = File::create(path)?;
    file.write_all(&flac_data)?;

    Ok(())
}

/// Encode stereo audio to FLAC in memory
pub fn encode_flac_memory(left: &[f32], right: &[f32], config: &FlacConfig) -> Result<Vec<u8>> {
    if left.len() != right.len() {
        return Err(ExportError::InvalidData(
            "Left and right channels have different lengths".into(),
        ));
    }

    // FLAC doesn't support 32-bit float
    if config.bit_depth == BitDepth::Float32 {
        return Err(ExportError::UnsupportedFormat(
            "FLAC does not support 32-bit float, use 16-bit or 24-bit".into(),
        ));
    }

    let bits_per_sample = match config.bit_depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => unreachable!(),
    };

    // Convert float samples to interleaved integers
    let interleaved = interleave_to_i32(left, right, config.bit_depth);

    // Create encoder config
    let encoder_config = EncoderConfig::default()
        .into_verified()
        .map_err(|e| ExportError::Encoding(format!("Invalid FLAC config: {:?}", e)))?;

    // Create source
    let source = MemSource::from_samples(
        &interleaved,
        config.channels as usize,
        bits_per_sample,
        config.sample_rate as usize,
    );

    // Encode
    let stream = encode_with_fixed_block_size(&encoder_config, source, config.block_size as usize)
        .map_err(|e| ExportError::Encoding(format!("FLAC encoding failed: {:?}", e)))?;

    // Write to ByteSink
    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| ExportError::Encoding(format!("Failed to write FLAC stream: {:?}", e)))?;

    Ok(sink.into_inner())
}

/// Encode mono audio to FLAC file
pub fn encode_flac_mono_file(samples: &[f32], path: &Path, config: &FlacConfig) -> Result<()> {
    let flac_data = encode_flac_mono_memory(samples, config)?;

    let mut file = File::create(path)?;
    file.write_all(&flac_data)?;

    Ok(())
}

/// Encode mono audio to FLAC in memory
pub fn encode_flac_mono_memory(samples: &[f32], config: &FlacConfig) -> Result<Vec<u8>> {
    // FLAC doesn't support 32-bit float
    if config.bit_depth == BitDepth::Float32 {
        return Err(ExportError::UnsupportedFormat(
            "FLAC does not support 32-bit float, use 16-bit or 24-bit".into(),
        ));
    }

    let bits_per_sample = match config.bit_depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => unreachable!(),
    };

    // Convert float samples to integers
    let int_samples: Vec<i32> = samples
        .iter()
        .map(|&s| float_to_i32(s, config.bit_depth))
        .collect();

    // Create encoder config
    let encoder_config = EncoderConfig::default()
        .into_verified()
        .map_err(|e| ExportError::Encoding(format!("Invalid FLAC config: {:?}", e)))?;

    // Create source
    let source = MemSource::from_samples(
        &int_samples,
        1,
        bits_per_sample,
        config.sample_rate as usize,
    );

    // Encode
    let stream = encode_with_fixed_block_size(&encoder_config, source, config.block_size as usize)
        .map_err(|e| ExportError::Encoding(format!("FLAC encoding failed: {:?}", e)))?;

    // Write to ByteSink
    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| ExportError::Encoding(format!("Failed to write FLAC stream: {:?}", e)))?;

    Ok(sink.into_inner())
}

/// Interleave stereo channels and convert to i32
fn interleave_to_i32(left: &[f32], right: &[f32], bit_depth: BitDepth) -> Vec<i32> {
    let mut interleaved = Vec::with_capacity(left.len() * 2);

    for i in 0..left.len() {
        interleaved.push(float_to_i32(left[i], bit_depth));
        interleaved.push(float_to_i32(right[i], bit_depth));
    }

    interleaved
}

/// Convert float sample to i32 with appropriate scaling
#[inline]
fn float_to_i32(sample: f32, bit_depth: BitDepth) -> i32 {
    let clamped = sample.clamp(-1.0, 1.0);
    match bit_depth {
        BitDepth::Int16 => (clamped * 32767.0) as i32,
        BitDepth::Int24 => (clamped * 8388607.0) as i32,
        BitDepth::Float32 => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flac_config_default() {
        let config = FlacConfig::default();
        assert_eq!(config.sample_rate, 44100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.compression_level, 5);
    }

    #[test]
    fn test_flac_config_stereo() {
        let config = FlacConfig::stereo(48000, BitDepth::Int24);
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn test_flac_rejects_32bit_float() {
        let left = vec![0.0; 100];
        let right = vec![0.0; 100];
        let config = FlacConfig {
            bit_depth: BitDepth::Float32,
            ..Default::default()
        };

        let result = encode_flac_memory(&left, &right, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_interleave_to_i32() {
        let left = vec![0.0, 1.0];
        let right = vec![0.5, -0.5];

        let interleaved = interleave_to_i32(&left, &right, BitDepth::Int16);

        assert_eq!(interleaved.len(), 4);
        assert_eq!(interleaved[0], 0); // left[0]
        assert_eq!(interleaved[1], 16383); // right[0] (0.5 * 32767 ≈ 16383)
        assert_eq!(interleaved[2], 32767); // left[1]
        assert_eq!(interleaved[3], -16383); // right[1]
    }

    #[test]
    fn test_mismatched_channel_lengths() {
        let left = vec![0.0, 0.5];
        let right = vec![0.0];
        let config = FlacConfig::default();

        let result = encode_flac_memory(&left, &right, &config);
        assert!(result.is_err());
    }
}
