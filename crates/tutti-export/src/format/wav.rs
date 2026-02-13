//! WAV format encoder.

use crate::dsp::{process_audio, stereo_to_mono};
use crate::error::{ExportError, Result};
use crate::export_builder::{ExportPhase, ExportProgress};
use crate::options::{BitDepth, ExportOptions};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::io::{Seek, Write};
use std::path::Path;

#[derive(Debug, Clone)]
struct WavConfig {
    sample_rate: u32,
    bit_depth: BitDepth,
    channels: u16,
}

pub(crate) fn export_wav(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
) -> Result<()> {
    let config = WavConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
    };

    let (left_proc, right_proc) = process_audio(left, right, options, config.sample_rate)?;

    if options.mono {
        encode_wav_mono_file(
            &stereo_to_mono(&left_proc, &right_proc),
            Path::new(path),
            &config,
        )
    } else {
        encode_wav_file(&left_proc, &right_proc, Path::new(path), &config)
    }
}

pub(crate) fn export_wav_with_progress(
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

    let config = WavConfig {
        sample_rate: options.output_sample_rate(),
        bit_depth: options.bit_depth,
        channels: if options.mono { 1 } else { 2 },
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
        encode_wav_mono_file(
            &stereo_to_mono(&left_proc, &right_proc),
            Path::new(path),
            &config,
        )
    } else {
        encode_wav_file(&left_proc, &right_proc, Path::new(path), &config)
    };

    on_progress(ExportProgress {
        phase: ExportPhase::Encoding,
        progress: 1.0,
    });

    result
}

fn encode_wav_file(left: &[f32], right: &[f32], path: &Path, config: &WavConfig) -> Result<()> {
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

fn encode_wav_mono_file(samples: &[f32], path: &Path, config: &WavConfig) -> Result<()> {
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

#[inline]
fn float_to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0) as i16
}

#[inline]
fn float_to_i24(sample: f32) -> i32 {
    (sample.clamp(-1.0, 1.0) * 8388607.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_float_to_i16() {
        assert_eq!(float_to_i16(0.0), 0);
        assert_eq!(float_to_i16(1.0), 32767);
        assert_eq!(float_to_i16(-1.0), -32767);
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
    fn test_export_wav_file() {
        let left = vec![0.0, 0.5, -0.5];
        let right = vec![0.1, -0.1, 0.0];
        let options = ExportOptions::default();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        export_wav(path.to_str().unwrap(), &left, &right, &options).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
    }
}
