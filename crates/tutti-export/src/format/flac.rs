//! FLAC format encoder.

use crate::dsp::{process_audio, stereo_to_mono};
use crate::error::{ExportError, Result};
use crate::export_builder::{ExportPhase, ExportProgress};
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

#[derive(Debug, Clone)]
struct FlacConfig {
    sample_rate: u32,
    bit_depth: BitDepth,
    channels: u16,
    block_size: u32,
}

impl Default for FlacConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44100,
            bit_depth: BitDepth::Int16,
            channels: 2,
            block_size: 4096,
        }
    }
}

pub(crate) fn export_flac(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
) -> Result<()> {
    let config = FlacConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
        ..Default::default()
    };

    let (left_proc, right_proc) = process_audio(left, right, options, config.sample_rate)?;

    if options.mono {
        encode_flac_mono_file(
            &stereo_to_mono(&left_proc, &right_proc),
            Path::new(path),
            &config,
        )
    } else {
        encode_flac_file(&left_proc, &right_proc, Path::new(path), &config)
    }
}

pub(crate) fn export_flac_with_progress(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
    on_progress: impl Fn(ExportProgress),
) -> Result<()> {
    on_progress(ExportProgress {
        phase: ExportPhase::Processing,
        progress: 0.0,
    });

    let config = FlacConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
        ..Default::default()
    };

    let (left_proc, right_proc) = process_audio(left, right, options, config.sample_rate)?;

    on_progress(ExportProgress {
        phase: ExportPhase::Processing,
        progress: 1.0,
    });

    on_progress(ExportProgress {
        phase: ExportPhase::Encoding,
        progress: 0.0,
    });

    let result = if options.mono {
        encode_flac_mono_file(
            &stereo_to_mono(&left_proc, &right_proc),
            Path::new(path),
            &config,
        )
    } else {
        encode_flac_file(&left_proc, &right_proc, Path::new(path), &config)
    };

    on_progress(ExportProgress {
        phase: ExportPhase::Encoding,
        progress: 1.0,
    });

    result
}

fn encode_flac_file(left: &[f32], right: &[f32], path: &Path, config: &FlacConfig) -> Result<()> {
    let flac_data = encode_flac_memory(left, right, config)?;
    let mut file = File::create(path)?;
    file.write_all(&flac_data)?;
    Ok(())
}

fn encode_flac_memory(left: &[f32], right: &[f32], config: &FlacConfig) -> Result<Vec<u8>> {
    if left.len() != right.len() {
        return Err(ExportError::InvalidData(
            "Left and right channels have different lengths".into(),
        ));
    }

    if config.bit_depth == BitDepth::Float32 {
        return Err(ExportError::UnsupportedFormat(
            "FLAC does not support 32-bit float".into(),
        ));
    }

    let bits_per_sample = match config.bit_depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => unreachable!(),
    };

    let interleaved = interleave_to_i32(left, right, config.bit_depth);

    let encoder_config = EncoderConfig::default()
        .into_verified()
        .map_err(|e| ExportError::Encoding(format!("Invalid FLAC config: {:?}", e)))?;

    let source = MemSource::from_samples(
        &interleaved,
        config.channels as usize,
        bits_per_sample,
        config.sample_rate as usize,
    );

    let stream = encode_with_fixed_block_size(&encoder_config, source, config.block_size as usize)
        .map_err(|e| ExportError::Encoding(format!("FLAC encoding failed: {:?}", e)))?;

    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| ExportError::Encoding(format!("Failed to write FLAC stream: {:?}", e)))?;

    Ok(sink.into_inner())
}

fn encode_flac_mono_file(samples: &[f32], path: &Path, config: &FlacConfig) -> Result<()> {
    let flac_data = encode_flac_mono_memory(samples, config)?;
    let mut file = File::create(path)?;
    file.write_all(&flac_data)?;
    Ok(())
}

fn encode_flac_mono_memory(samples: &[f32], config: &FlacConfig) -> Result<Vec<u8>> {
    if config.bit_depth == BitDepth::Float32 {
        return Err(ExportError::UnsupportedFormat(
            "FLAC does not support 32-bit float".into(),
        ));
    }

    let bits_per_sample = match config.bit_depth {
        BitDepth::Int16 => 16,
        BitDepth::Int24 => 24,
        BitDepth::Float32 => unreachable!(),
    };

    let int_samples: Vec<i32> = samples
        .iter()
        .map(|&s| float_to_i32(s, config.bit_depth))
        .collect();

    let encoder_config = EncoderConfig::default()
        .into_verified()
        .map_err(|e| ExportError::Encoding(format!("Invalid FLAC config: {:?}", e)))?;

    let source = MemSource::from_samples(
        &int_samples,
        1,
        bits_per_sample,
        config.sample_rate as usize,
    );

    let stream = encode_with_fixed_block_size(&encoder_config, source, config.block_size as usize)
        .map_err(|e| ExportError::Encoding(format!("FLAC encoding failed: {:?}", e)))?;

    let mut sink = ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| ExportError::Encoding(format!("Failed to write FLAC stream: {:?}", e)))?;

    Ok(sink.into_inner())
}

fn interleave_to_i32(left: &[f32], right: &[f32], bit_depth: BitDepth) -> Vec<i32> {
    let mut interleaved = Vec::with_capacity(left.len() * 2);
    for i in 0..left.len() {
        interleaved.push(float_to_i32(left[i], bit_depth));
        interleaved.push(float_to_i32(right[i], bit_depth));
    }
    interleaved
}

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
    fn test_interleave_to_i32() {
        let left = vec![0.0, 1.0];
        let right = vec![0.5, -0.5];
        let interleaved = interleave_to_i32(&left, &right, BitDepth::Int16);

        assert_eq!(interleaved.len(), 4);
        assert_eq!(interleaved[0], 0);
        assert_eq!(interleaved[1], 16383);
        assert_eq!(interleaved[2], 32767);
        assert_eq!(interleaved[3], -16383);
    }

    #[test]
    fn test_export_flac_rejects_32bit_float() {
        let left = vec![0.0; 100];
        let right = vec![0.0; 100];
        let mut options = ExportOptions::default();
        options.bit_depth = BitDepth::Float32;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.flac");
        let result = export_flac(path.to_str().unwrap(), &left, &right, &options);
        assert!(result.is_err());
    }
}
