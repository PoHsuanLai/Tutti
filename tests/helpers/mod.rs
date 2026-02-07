//! Test helpers and fixtures for Tutti integration tests
//!
//! Inspired by Ardour (CppUnit fixtures, dummy backend, reference data)
//! and Zrythm (mock audio I/O, manual cycle control, round-trip testing).
//!
//! ## Tolerance Levels
//!
//! Use the appropriate tolerance from [`tolerances`] module:
//! - `FLOAT_EPSILON` (1e-6): Exact operations (passthrough, unity gain)
//! - `DSP_EPSILON` (1e-4): DSP processing (filters, oscillators)
//! - `PERCEPTUAL_EPSILON` (0.001): Perceptual equivalence (-60dB)
//! - `SILENCE_THRESHOLD` (0.0001): Silence detection (-80dB)

pub mod tolerances;

use std::path::{Path, PathBuf};
use tutti::prelude::*;

/// Default test sample rate (matches common hardware)
pub const TEST_SAMPLE_RATE: f64 = 48000.0;

/// Standard buffer size for deterministic testing
pub const TEST_BUFFER_SIZE: usize = 512;

/// Create a basic test engine with minimal configuration.
/// Avoids hardware audio I/O for CI environments.
pub fn test_engine() -> TuttiEngine {
    TuttiEngine::builder()
        .sample_rate(TEST_SAMPLE_RATE)
        .build()
        .expect("Failed to create test engine")
}

/// Create a test engine with specific sample rate.
pub fn test_engine_with_sr(sample_rate: f64) -> TuttiEngine {
    TuttiEngine::builder()
        .sample_rate(sample_rate)
        .build()
        .expect("Failed to create test engine")
}

/// Generate a test signal: sine wave at given frequency for specified samples.
pub fn generate_sine(frequency: f64, sample_rate: f64, num_samples: usize) -> Vec<f32> {
    (0..num_samples)
        .map(|i| {
            let t = i as f64 / sample_rate;
            (2.0 * std::f64::consts::PI * frequency * t).sin() as f32
        })
        .collect()
}

/// Generate silence (zero samples).
pub fn generate_silence(num_samples: usize) -> Vec<f32> {
    vec![0.0; num_samples]
}

/// Generate white noise (random samples in -1..1).
pub fn generate_noise(num_samples: usize, seed: u64) -> Vec<f32> {
    // Simple LCG for reproducible "random" noise
    let mut rng = seed;
    (0..num_samples)
        .map(|_| {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((rng >> 33) as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect()
}

/// Calculate RMS of a signal.
pub fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Calculate peak amplitude of a signal.
pub fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .map(|s| s.abs())
        .fold(0.0_f32, |a, b| a.max(b))
}

/// Check if two signals are approximately equal within tolerance.
pub fn signals_approx_equal(a: &[f32], b: &[f32], tolerance: f32) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= tolerance)
}

/// Assert that a signal is approximately silent (all values near zero).
pub fn assert_silence(samples: &[f32], tolerance: f32) {
    let max = peak(samples);
    assert!(
        max <= tolerance,
        "Expected silence, but peak amplitude was {}",
        max
    );
}

/// Assert that a signal has content (not silent).
pub fn assert_has_audio(samples: &[f32], min_rms: f32) {
    let r = rms(samples);
    assert!(
        r >= min_rms,
        "Expected audio content with RMS >= {}, but RMS was {}",
        min_rms,
        r
    );
}

/// Transport test helper - wait for transport to reach a specific beat.
pub fn wait_for_beat(engine: &TuttiEngine, target_beat: f64, max_wait_ms: u64) -> bool {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(max_wait_ms);

    while start.elapsed() < timeout {
        if engine.transport().current_beat() >= target_beat {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

/// Transport test helper - wait for transport state.
pub fn wait_for_state(
    engine: &TuttiEngine,
    expected_playing: bool,
    max_wait_ms: u64,
) -> bool {
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(max_wait_ms);

    while start.elapsed() < timeout {
        if engine.transport().is_playing() == expected_playing {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

// =============================================================================
// Deterministic Signal Generators (Ardour-style)
// =============================================================================

/// Generate an integer staircase signal [0, 1, 2, ..., n-1] as f32.
///
/// Used for exact sample verification (Ardour pattern). Each sample
/// equals its index, allowing precise verification of signal routing.
///
/// # Example
/// ```ignore
/// let staircase = generate_integer_staircase(1024);
/// assert_eq!(staircase[0], 0.0);
/// assert_eq!(staircase[100], 100.0);
/// assert_eq!(staircase[1023], 1023.0);
/// ```
pub fn generate_integer_staircase(num_samples: usize) -> Vec<f32> {
    (0..num_samples).map(|i| i as f32).collect()
}

/// Generate a normalized staircase signal in range [-1, 1].
///
/// Unlike integer staircase, this is suitable for audio processing
/// tests where signals should be in the standard audio range.
pub fn generate_normalized_staircase(num_samples: usize) -> Vec<f32> {
    if num_samples <= 1 {
        return vec![0.0; num_samples];
    }
    let max = (num_samples - 1) as f32;
    (0..num_samples)
        .map(|i| (i as f32 / max) * 2.0 - 1.0)
        .collect()
}

/// Generate an impulse signal (single sample at 1.0, rest zeros).
///
/// Useful for testing latency, delay lines, and impulse responses.
///
/// # Arguments
/// * `num_samples` - Total length of the signal
/// * `position` - Sample index where the impulse occurs
pub fn generate_impulse(num_samples: usize, position: usize) -> Vec<f32> {
    let mut samples = vec![0.0; num_samples];
    if position < num_samples {
        samples[position] = 1.0;
    }
    samples
}

/// Generate a DC offset signal (constant value).
///
/// Useful for testing DC blocking, offset removal, and constant gain.
pub fn generate_dc(value: f32, num_samples: usize) -> Vec<f32> {
    vec![value; num_samples]
}

/// Generate a linear ramp from start to end value.
///
/// Useful for testing gain automation and linear interpolation.
pub fn generate_ramp(start: f32, end: f32, num_samples: usize) -> Vec<f32> {
    if num_samples <= 1 {
        return vec![start; num_samples];
    }
    let step = (end - start) / (num_samples - 1) as f32;
    (0..num_samples)
        .map(|i| start + step * i as f32)
        .collect()
}

// =============================================================================
// Audio Comparison Utilities (Zrythm-style)
// =============================================================================

/// Result of comparing two audio buffers.
#[derive(Debug, Clone)]
pub struct AudioComparisonResult {
    /// Whether all samples are within tolerance.
    pub equal: bool,
    /// Maximum absolute difference between any two samples.
    pub max_diff: f32,
    /// Mean absolute difference across all samples.
    pub mean_diff: f32,
    /// Index of first sample that exceeds tolerance (if any).
    pub first_diff_sample: Option<usize>,
    /// Number of samples that exceed tolerance.
    pub num_diffs: usize,
}

/// Compare two audio buffers with epsilon tolerance (Zrythm-style).
///
/// Returns detailed comparison results including max/mean differences
/// and the location of the first mismatch.
pub fn compare_audio(a: &[f32], b: &[f32], epsilon: f32) -> AudioComparisonResult {
    if a.len() != b.len() {
        return AudioComparisonResult {
            equal: false,
            max_diff: f32::MAX,
            mean_diff: f32::MAX,
            first_diff_sample: Some(0),
            num_diffs: std::cmp::max(a.len(), b.len()),
        };
    }

    if a.is_empty() {
        return AudioComparisonResult {
            equal: true,
            max_diff: 0.0,
            mean_diff: 0.0,
            first_diff_sample: None,
            num_diffs: 0,
        };
    }

    let mut max_diff: f32 = 0.0;
    let mut sum_diff: f32 = 0.0;
    let mut first_diff: Option<usize> = None;
    let mut num_diffs = 0;

    for (i, (&x, &y)) in a.iter().zip(b.iter()).enumerate() {
        let diff = (x - y).abs();
        max_diff = max_diff.max(diff);
        sum_diff += diff;
        if diff > epsilon {
            num_diffs += 1;
            if first_diff.is_none() {
                first_diff = Some(i);
            }
        }
    }

    AudioComparisonResult {
        equal: num_diffs == 0,
        max_diff,
        mean_diff: sum_diff / a.len() as f32,
        first_diff_sample: first_diff,
        num_diffs,
    }
}

/// Check if audio is silent (all samples below threshold).
///
/// This is the Zrythm-style `audio_file_is_silent` pattern.
pub fn is_silent(samples: &[f32], threshold: f32) -> bool {
    samples.iter().all(|&s| s.abs() <= threshold)
}

/// Check staircase signal integrity (Ardour-style).
///
/// Verifies each sample equals its expected integer index within tolerance.
pub fn check_staircase(samples: &[f32], expected_len: usize, tolerance: f32) -> bool {
    if samples.len() != expected_len {
        return false;
    }
    samples
        .iter()
        .enumerate()
        .all(|(i, &s)| (s - i as f32).abs() <= tolerance)
}

/// Check if two stereo audio buffers are equal within tolerance.
pub fn stereo_equal(
    left_a: &[f32],
    right_a: &[f32],
    left_b: &[f32],
    right_b: &[f32],
    epsilon: f32,
) -> bool {
    compare_audio(left_a, left_b, epsilon).equal && compare_audio(right_a, right_b, epsilon).equal
}

// =============================================================================
// Assertion Functions
// =============================================================================

/// Assert two signals are equal within tolerance, with detailed error message.
pub fn assert_signals_equal(a: &[f32], b: &[f32], epsilon: f32, context: &str) {
    let result = compare_audio(a, b, epsilon);
    assert!(
        result.equal,
        "{}: Signals differ - first diff at sample {:?}, max_diff={:.6}, mean_diff={:.6}, num_diffs={}",
        context,
        result.first_diff_sample,
        result.max_diff,
        result.mean_diff,
        result.num_diffs
    );
}

/// Assert staircase signal is valid (Ardour pattern).
pub fn assert_staircase_valid(samples: &[f32], expected_len: usize, tolerance: f32) {
    assert_eq!(
        samples.len(),
        expected_len,
        "Staircase length mismatch: got {}, expected {}",
        samples.len(),
        expected_len
    );
    for (i, &s) in samples.iter().enumerate() {
        assert!(
            (s - i as f32).abs() <= tolerance,
            "Staircase mismatch at sample {}: expected {}, got {} (diff={})",
            i,
            i,
            s,
            (s - i as f32).abs()
        );
    }
}

/// Assert signal is silent within threshold.
pub fn assert_is_silent(samples: &[f32], threshold: f32, context: &str) {
    let max_val = peak(samples);
    assert!(
        max_val <= threshold,
        "{}: Expected silence (threshold {}), but peak was {}",
        context,
        threshold,
        max_val
    );
}

/// Assert signal is NOT silent (has content above threshold).
pub fn assert_not_silent(samples: &[f32], min_peak: f32, context: &str) {
    let max_val = peak(samples);
    assert!(
        max_val >= min_peak,
        "{}: Expected audio (min_peak {}), but peak was only {}",
        context,
        min_peak,
        max_val
    );
}

// =============================================================================
// Reference File I/O
// =============================================================================

/// Get path to test data directory.
pub fn test_data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
}

/// Load reference audio from WAV file in test data directory.
///
/// Returns `(left_channel, right_channel, sample_rate)`.
#[cfg(feature = "wav")]
pub fn load_reference_wav(name: &str) -> Result<(Vec<f32>, Vec<f32>, u32), String> {
    let path = test_data_dir().join(name);
    load_wav_file(&path)
}

/// Load WAV file into stereo buffers.
///
/// Returns `(left_channel, right_channel, sample_rate)`.
#[cfg(feature = "wav")]
pub fn load_wav_file(path: &Path) -> Result<(Vec<f32>, Vec<f32>, u32), String> {
    use hound::WavReader;

    let reader = WavReader::open(path).map_err(|e| format!("Failed to open WAV '{}': {}", path.display(), e))?;

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;
    let channels = spec.channels as usize;

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to read float samples: {}", e))?,
        hound::SampleFormat::Int => {
            let bits = spec.bits_per_sample;
            let max_val = (1i32 << (bits - 1)) as f32;
            reader
                .into_samples::<i32>()
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("Failed to read int samples: {}", e))?
                .into_iter()
                .map(|s| s as f32 / max_val)
                .collect()
        }
    };

    // Deinterleave to stereo
    let (left, right) = if channels >= 2 {
        let mut l = Vec::with_capacity(samples.len() / channels);
        let mut r = Vec::with_capacity(samples.len() / channels);
        for chunk in samples.chunks(channels) {
            l.push(chunk[0]);
            r.push(chunk.get(1).copied().unwrap_or(chunk[0]));
        }
        (l, r)
    } else {
        // Mono: duplicate to stereo
        (samples.clone(), samples)
    };

    Ok((left, right, sample_rate))
}

/// Save stereo audio to WAV file.
#[cfg(feature = "wav")]
pub fn save_wav_file(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    use hound::{WavSpec, WavWriter};

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };

    let mut writer =
        WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {}", e))?;

    let len = std::cmp::min(left.len(), right.len());
    for i in 0..len {
        writer
            .write_sample(left[i])
            .map_err(|e| format!("Write error: {}", e))?;
        writer
            .write_sample(right[i])
            .map_err(|e| format!("Write error: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Finalize error: {}", e))?;

    Ok(())
}

/// Save reference audio to test data directory.
#[cfg(feature = "wav")]
pub fn save_reference_wav(
    name: &str,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    let path = test_data_dir().join(name);
    save_wav_file(&path, left, right, sample_rate)
}

/// Save stereo audio to 16-bit PCM WAV file (for maximum compatibility).
/// Use this when the WAV needs to be loaded by external tools or symphonia.
#[cfg(feature = "wav")]
pub fn save_wav_file_pcm16(
    path: &Path,
    left: &[f32],
    right: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    use hound::{WavSpec, WavWriter};

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let spec = WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer =
        WavWriter::create(path, spec).map_err(|e| format!("Failed to create WAV: {}", e))?;

    let len = std::cmp::min(left.len(), right.len());
    for i in 0..len {
        // Convert f32 [-1.0, 1.0] to i16 [-32768, 32767]
        let l = (left[i].clamp(-1.0, 1.0) * 32767.0) as i16;
        let r = (right[i].clamp(-1.0, 1.0) * 32767.0) as i16;
        writer
            .write_sample(l)
            .map_err(|e| format!("Write error: {}", e))?;
        writer
            .write_sample(r)
            .map_err(|e| format!("Write error: {}", e))?;
    }

    writer
        .finalize()
        .map_err(|e| format!("Finalize error: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_sine() {
        let samples = generate_sine(440.0, 44100.0, 44100);
        assert_eq!(samples.len(), 44100);
        // Check it's not silent
        assert!(rms(&samples) > 0.5);
        // Check it's normalized
        assert!(peak(&samples) <= 1.0);
    }

    #[test]
    fn test_rms_calculation() {
        // Full-scale sine wave has RMS of ~0.707
        let samples = generate_sine(440.0, 44100.0, 44100);
        let r = rms(&samples);
        assert!((r - 0.707).abs() < 0.01);
    }

    #[test]
    fn test_signals_approx_equal() {
        let a = vec![0.0, 0.5, 1.0];
        let b = vec![0.001, 0.501, 0.999];
        assert!(signals_approx_equal(&a, &b, 0.01));
        assert!(!signals_approx_equal(&a, &b, 0.0001));
    }
}
