//! Transient Detection for audio analysis
//!
//! Implements onset detection for identifying transients in audio.
//! Uses spectral flux algorithm with adaptive thresholding.
//!
//! ## Use Cases
//!
//! - Beat detection for tempo analysis
//! - Beat slicing for sample manipulation
//! - Quantizing audio to grid
//! - Triggering events on transients

use rustfft::{num_complex::Complex, FftPlanner};

/// Default FFT size for analysis
const DEFAULT_FFT_SIZE: usize = 1024;

/// Default hop size (samples between analysis frames)
const DEFAULT_HOP_SIZE: usize = 512;

/// A detected transient/onset
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct Transient {
    /// Sample position of the transient
    pub sample_position: usize,
    /// Time position in seconds
    pub time: f64,
    /// Strength/confidence of detection (0.0 - 1.0)
    pub strength: f32,
}

/// Transient detection algorithm type
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum DetectionMethod {
    /// Spectral flux (default, good for most audio)
    #[default]
    SpectralFlux,
    /// High-frequency content (good for percussive material)
    HighFrequencyContent,
    /// Energy-based (simple, fast)
    Energy,
    /// Complex domain (phase-based, very accurate)
    ComplexDomain,
}

/// Transient detector for audio analysis
pub struct TransientDetector {
    /// Sample rate
    sample_rate: f64,
    /// FFT size
    fft_size: usize,
    /// Hop size (samples between frames)
    hop_size: usize,
    /// Detection threshold (0.0 - 1.0)
    threshold: f32,
    /// Sensitivity (higher = more detections)
    sensitivity: f32,
    /// Minimum gap between detections in samples
    min_gap: usize,
    /// Detection method
    method: DetectionMethod,
    /// FFT planner
    fft_planner: FftPlanner<f32>,
    /// Window function
    window: Vec<f32>,
    /// Previous magnitude spectrum (for flux calculation)
    prev_magnitudes: Vec<f32>,
}

impl TransientDetector {
    /// Create a new transient detector
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(sample_rate: f64) -> Self {
        Self::with_params(sample_rate, DEFAULT_FFT_SIZE, DEFAULT_HOP_SIZE)
    }

    /// Create with custom FFT and hop size
    pub fn with_params(sample_rate: f64, fft_size: usize, hop_size: usize) -> Self {
        let fft_size = fft_size.next_power_of_two();
        let window = Self::create_hann_window(fft_size);

        Self {
            sample_rate,
            fft_size,
            hop_size,
            threshold: 0.3,
            sensitivity: 1.0,
            min_gap: (sample_rate * 0.05) as usize, // 50ms minimum gap
            method: DetectionMethod::SpectralFlux,
            fft_planner: FftPlanner::new(),
            window,
            prev_magnitudes: vec![0.0; fft_size / 2],
        }
    }

    /// Set detection threshold (0.0 - 1.0)
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold.clamp(0.0, 1.0);
    }

    /// Set sensitivity (0.1 - 10.0, higher = more detections)
    pub fn set_sensitivity(&mut self, sensitivity: f32) {
        self.sensitivity = sensitivity.clamp(0.1, 10.0);
    }

    /// Set minimum gap between detections in milliseconds
    pub fn set_min_gap_ms(&mut self, gap_ms: f32) {
        self.min_gap = (gap_ms / 1000.0 * self.sample_rate as f32) as usize;
    }

    /// Set detection method
    pub fn set_method(&mut self, method: DetectionMethod) {
        self.method = method;
        self.reset();
    }

    /// Reset detector state
    pub fn reset(&mut self) {
        self.prev_magnitudes.fill(0.0);
    }

    /// Create Hann window
    fn create_hann_window(size: usize) -> Vec<f32> {
        (0..size)
            .map(|i| {
                let angle = 2.0 * core::f32::consts::PI * i as f32 / (size - 1) as f32;
                0.5 * (1.0 - angle.cos())
            })
            .collect()
    }

    /// Analyze audio and detect transients
    ///
    /// # Arguments
    /// * `samples` - Mono audio samples
    ///
    /// # Returns
    /// Vector of detected transients sorted by time
    pub fn detect(&mut self, samples: &[f32]) -> Vec<Transient> {
        if samples.len() < self.fft_size {
            return Vec::new();
        }

        let mut detection_function = Vec::new();

        // Process frames
        let num_frames = (samples.len() - self.fft_size) / self.hop_size + 1;

        for frame_idx in 0..num_frames {
            let start = frame_idx * self.hop_size;
            let frame = &samples[start..start + self.fft_size];

            let value = match self.method {
                DetectionMethod::SpectralFlux => self.spectral_flux(frame),
                DetectionMethod::HighFrequencyContent => self.high_frequency_content(frame),
                DetectionMethod::Energy => self.energy(frame),
                DetectionMethod::ComplexDomain => self.complex_domain(frame),
            };

            detection_function.push((start, value));
        }

        // Apply adaptive thresholding
        let peaks = self.find_peaks(&detection_function);

        // Convert peaks to transients with minimum gap enforcement
        let mut transients = Vec::new();
        let mut last_position = 0usize;

        for (position, strength) in peaks {
            if position >= last_position + self.min_gap || last_position == 0 {
                transients.push(Transient {
                    sample_position: position,
                    time: position as f64 / self.sample_rate,
                    strength,
                });
                last_position = position;
            }
        }

        transients
    }

    /// Spectral flux detection function
    fn spectral_flux(&mut self, frame: &[f32]) -> f32 {
        let mut buffer: Vec<Complex<f32>> = frame
            .iter()
            .zip(self.window.iter())
            .map(|(s, w)| Complex::new(s * w, 0.0))
            .collect();

        let fft = self.fft_planner.plan_fft_forward(self.fft_size);
        fft.process(&mut buffer);

        // Calculate magnitudes
        let magnitudes: Vec<f32> = buffer[..self.fft_size / 2]
            .iter()
            .map(|c| c.norm())
            .collect();

        // Calculate spectral flux (sum of positive differences)
        let mut flux = 0.0;
        for (i, &mag) in magnitudes.iter().enumerate() {
            let diff = mag - self.prev_magnitudes[i];
            if diff > 0.0 {
                flux += diff;
            }
        }

        // Update previous magnitudes
        self.prev_magnitudes.copy_from_slice(&magnitudes);

        flux * self.sensitivity
    }

    /// High-frequency content detection function
    fn high_frequency_content(&mut self, frame: &[f32]) -> f32 {
        let mut buffer: Vec<Complex<f32>> = frame
            .iter()
            .zip(self.window.iter())
            .map(|(s, w)| Complex::new(s * w, 0.0))
            .collect();

        let fft = self.fft_planner.plan_fft_forward(self.fft_size);
        fft.process(&mut buffer);

        // Weight bins by frequency (higher bins weighted more)
        let mut hfc = 0.0;
        for (i, c) in buffer[..self.fft_size / 2].iter().enumerate() {
            let weight = (i + 1) as f32;
            hfc += weight * c.norm_sqr();
        }

        hfc.sqrt() * self.sensitivity * 0.01
    }

    /// Energy-based detection function
    fn energy(&self, frame: &[f32]) -> f32 {
        let energy: f32 = frame.iter().map(|s| s * s).sum();
        energy.sqrt() * self.sensitivity
    }

    /// Complex domain detection function (phase-based)
    fn complex_domain(&mut self, frame: &[f32]) -> f32 {
        let mut buffer: Vec<Complex<f32>> = frame
            .iter()
            .zip(self.window.iter())
            .map(|(s, w)| Complex::new(s * w, 0.0))
            .collect();

        let fft = self.fft_planner.plan_fft_forward(self.fft_size);
        fft.process(&mut buffer);

        // Calculate magnitudes and sum differences
        let mut value = 0.0;
        for (i, c) in buffer[..self.fft_size / 2].iter().enumerate() {
            let mag = c.norm();
            let diff = (mag - self.prev_magnitudes[i]).abs();
            value += diff * diff;
        }

        // Update previous magnitudes
        for (i, c) in buffer[..self.fft_size / 2].iter().enumerate() {
            self.prev_magnitudes[i] = c.norm();
        }

        value.sqrt() * self.sensitivity
    }

    /// Find peaks in detection function using adaptive threshold
    fn find_peaks(&self, detection_fn: &[(usize, f32)]) -> Vec<(usize, f32)> {
        if detection_fn.is_empty() {
            return Vec::new();
        }

        let mut peaks = Vec::new();

        // Calculate adaptive threshold (single-pass: sum, sum_sq, max)
        let len = detection_fn.len() as f32;
        let (sum, sum_sq, max_val) = detection_fn
            .iter()
            .fold((0.0f32, 0.0f32, 0.0f32), |(s, sq, mx), &(_, v)| {
                (s + v, sq + v * v, mx.max(v))
            });
        let mean = sum / len;
        let variance = sum_sq / len - mean * mean;
        let std_dev = variance.sqrt();

        let adaptive_threshold = mean + std_dev * self.threshold * 3.0;

        // Find local maxima above threshold
        for i in 1..detection_fn.len() - 1 {
            let (pos, val) = detection_fn[i];
            let (_, prev_val) = detection_fn[i - 1];
            let (_, next_val) = detection_fn[i + 1];

            if val > prev_val && val > next_val && val > adaptive_threshold {
                let strength = if max_val > 0.0 {
                    (val / max_val).min(1.0)
                } else {
                    0.0
                };

                peaks.push((pos, strength));
            }
        }

        peaks
    }

    /// Clean up transient list by removing closely spaced detections
    pub fn cleanup_transients(transients: &mut Vec<Transient>, min_gap_seconds: f64) {
        if transients.len() < 2 {
            return;
        }

        let mut i = 1;
        while i < transients.len() {
            if transients[i].time - transients[i - 1].time < min_gap_seconds {
                // Keep the stronger one
                if transients[i].strength > transients[i - 1].strength {
                    transients.remove(i - 1);
                } else {
                    transients.remove(i);
                }
            } else {
                i += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_test_signal(sample_rate: f64, duration: f64, transient_times: &[f64]) -> Vec<f32> {
        let num_samples = (sample_rate * duration) as usize;
        let mut samples = vec![0.0f32; num_samples];

        for &time in transient_times {
            let pos = (time * sample_rate) as usize;
            if pos < num_samples {
                for i in 0..50.min(num_samples - pos) {
                    let decay = (-0.1 * i as f32).exp();
                    samples[pos + i] += decay * 0.8;
                }
            }
        }

        samples
    }

    #[test]
    fn test_detector_creation() {
        let detector = TransientDetector::new(44100.0);
        assert_eq!(detector.fft_size, DEFAULT_FFT_SIZE);
        assert_eq!(detector.hop_size, DEFAULT_HOP_SIZE);
    }

    #[test]
    fn test_detect_simple_transients() {
        let sample_rate = 44100.0;
        let transient_times = vec![0.1, 0.3, 0.5, 0.7];
        let samples = generate_test_signal(sample_rate, 1.0, &transient_times);

        let mut detector = TransientDetector::new(sample_rate);
        detector.set_threshold(0.2);
        detector.set_sensitivity(2.0);

        let detected = detector.detect(&samples);

        assert!(!detected.is_empty(), "Should detect at least one transient");

        for transient in &detected {
            assert!(transient.time >= 0.0 && transient.time <= 1.0);
            assert!(transient.strength >= 0.0 && transient.strength <= 1.0);
        }
    }

    #[test]
    fn test_detection_methods() {
        let sample_rate = 44100.0;
        let samples = generate_test_signal(sample_rate, 0.5, &[0.1, 0.25]);

        for method in [
            DetectionMethod::SpectralFlux,
            DetectionMethod::HighFrequencyContent,
            DetectionMethod::Energy,
            DetectionMethod::ComplexDomain,
        ] {
            let mut detector = TransientDetector::new(sample_rate);
            detector.set_method(method);
            detector.set_threshold(0.2);

            let _detected = detector.detect(&samples);
        }
    }
}
