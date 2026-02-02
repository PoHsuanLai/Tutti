//! Audio resampling using rubato
//!
//! Provides high-quality sample rate conversion with SIMD optimization.

use crate::error::{ExportError, Result};
use rubato::{FftFixedIn, Resampler};

/// Resampling quality presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResampleQuality {
    /// Fast resampling (lower quality)
    Fast,
    /// Balanced quality/speed (default)
    #[default]
    Medium,
    /// High quality
    High,
    /// Best quality (slowest)
    Best,
}

impl ResampleQuality {
    /// Get the chunk size for this quality level
    fn chunk_size(&self) -> usize {
        match self {
            ResampleQuality::Fast => 512,
            ResampleQuality::Medium => 1024,
            ResampleQuality::High => 2048,
            ResampleQuality::Best => 4096,
        }
    }

    /// Get the sub-chunks for this quality level
    fn sub_chunks(&self) -> usize {
        match self {
            ResampleQuality::Fast => 1,
            ResampleQuality::Medium => 2,
            ResampleQuality::High => 4,
            ResampleQuality::Best => 8,
        }
    }
}

/// Resample stereo audio to a new sample rate
///
/// # Arguments
/// * `left` - Left channel samples
/// * `right` - Right channel samples
/// * `source_rate` - Source sample rate in Hz
/// * `target_rate` - Target sample rate in Hz
/// * `quality` - Resampling quality preset
///
/// # Returns
/// Resampled (left, right) channels
pub fn resample_stereo(
    left: &[f32],
    right: &[f32],
    source_rate: u32,
    target_rate: u32,
    quality: ResampleQuality,
) -> Result<(Vec<f32>, Vec<f32>)> {
    if source_rate == target_rate {
        // No resampling needed
        return Ok((left.to_vec(), right.to_vec()));
    }

    if left.len() != right.len() {
        return Err(ExportError::InvalidData(
            "Left and right channels have different lengths".into(),
        ));
    }

    let chunk_size = quality.chunk_size();
    let sub_chunks = quality.sub_chunks();

    // Create resampler
    let mut resampler = FftFixedIn::<f32>::new(
        source_rate as usize,
        target_rate as usize,
        chunk_size,
        sub_chunks,
        2, // stereo
    )?;

    // Prepare input channels
    let input_frames = left.len();
    let expected_output_frames =
        (input_frames as f64 * target_rate as f64 / source_rate as f64).ceil() as usize;

    let mut output_left = Vec::with_capacity(expected_output_frames + chunk_size);
    let mut output_right = Vec::with_capacity(expected_output_frames + chunk_size);

    // Process in chunks
    let mut pos = 0;
    while pos < input_frames {
        let remaining = input_frames - pos;
        let frames_to_process = remaining.min(chunk_size);

        // Check if we need the exact number of frames
        let input_frames_needed = resampler.input_frames_next();
        let actual_frames = if remaining < input_frames_needed {
            // Pad with zeros for final chunk
            input_frames_needed
        } else {
            frames_to_process.max(input_frames_needed)
        };

        // Prepare input buffers
        let mut chunk_left = vec![0.0f32; actual_frames];
        let mut chunk_right = vec![0.0f32; actual_frames];

        let copy_frames = frames_to_process.min(remaining);
        chunk_left[..copy_frames].copy_from_slice(&left[pos..pos + copy_frames]);
        chunk_right[..copy_frames].copy_from_slice(&right[pos..pos + copy_frames]);

        let input_channels = vec![chunk_left, chunk_right];

        // Resample
        let output = resampler.process(&input_channels, None)?;

        // Collect output
        output_left.extend_from_slice(&output[0]);
        output_right.extend_from_slice(&output[1]);

        pos += actual_frames;
    }

    // Trim to expected length (removing padding artifacts)
    let final_length = expected_output_frames.min(output_left.len());
    output_left.truncate(final_length);
    output_right.truncate(final_length);

    Ok((output_left, output_right))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_resample_needed() {
        let left = vec![1.0, 2.0, 3.0];
        let right = vec![4.0, 5.0, 6.0];

        let (out_l, out_r) =
            resample_stereo(&left, &right, 44100, 44100, ResampleQuality::Fast).unwrap();

        assert_eq!(out_l, left);
        assert_eq!(out_r, right);
    }

    #[test]
    fn test_resample_upsample() {
        // Generate a simple sine wave at 1000 Hz
        let sample_rate = 44100;
        let target_rate = 48000;
        let duration_samples = 4410; // 0.1 seconds

        let left: Vec<f32> = (0..duration_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = left.clone();

        let (out_l, out_r) = resample_stereo(
            &left,
            &right,
            sample_rate,
            target_rate,
            ResampleQuality::Medium,
        )
        .unwrap();

        // Check output length is approximately correct
        let expected_length =
            (duration_samples as f64 * target_rate as f64 / sample_rate as f64) as usize;
        assert!(
            (out_l.len() as i32 - expected_length as i32).abs() < 100,
            "Output length {} differs too much from expected {}",
            out_l.len(),
            expected_length
        );
        assert_eq!(out_l.len(), out_r.len());
    }

    #[test]
    fn test_resample_downsample() {
        let sample_rate = 96000;
        let target_rate = 44100;
        let duration_samples = 9600; // 0.1 seconds

        let left: Vec<f32> = (0..duration_samples)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = left.clone();

        let (out_l, _out_r) = resample_stereo(
            &left,
            &right,
            sample_rate,
            target_rate,
            ResampleQuality::High,
        )
        .unwrap();

        // Check output length is approximately correct
        let expected_length =
            (duration_samples as f64 * target_rate as f64 / sample_rate as f64) as usize;
        assert!(
            (out_l.len() as i32 - expected_length as i32).abs() < 100,
            "Output length {} differs too much from expected {}",
            out_l.len(),
            expected_length
        );
    }

    #[test]
    fn test_mismatched_channel_lengths() {
        let left = vec![1.0, 2.0, 3.0];
        let right = vec![4.0, 5.0];

        let result = resample_stereo(&left, &right, 44100, 48000, ResampleQuality::Fast);
        assert!(result.is_err());
    }
}
