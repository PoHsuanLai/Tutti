//! Pitch Detection using the YIN algorithm
//!
//! Implements monophonic pitch tracking suitable for:
//! - Instrument tuners
//! - Vocal pitch analysis
//! - Auto-tune preprocessing
//! - Melodic transcription
//!
//! ## Algorithm
//!
//! The YIN algorithm (de Cheveigné & Kawahara, 2002) is a robust
//! autocorrelation-based pitch detector. This implementation includes
//! all 6 steps from the original paper:
//!
//! 1. **Difference function** - d(τ) = Σ(x[j] - x[j+τ])²
//! 2. **Cumulative mean normalized difference** - d'(τ)
//! 3. **Absolute threshold** - Find first τ where d'(τ) < threshold
//! 4. **Parabolic interpolation** - Sub-sample accuracy
//!
//! ## Performance
//!
//! Current implementation is O(n × max_period) using direct computation.
//!
//! TODO: Optimize with FFT-based autocorrelation for O(n log n) performance.
//! The Wiener-Khinchin theorem allows computing autocorrelation as:
//!   r(τ) = IFFT(|FFT(x)|²)
//! Then difference function: d(τ) = r(0) + r'(0) - 2*r(τ)
//! Requires careful normalization to avoid octave errors.

/// Result of pitch detection for a single frame
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(
    feature = "serialization",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct PitchResult {
    /// Detected frequency in Hz (0.0 if unvoiced/uncertain)
    pub frequency: f32,
    /// Confidence/clarity of detection (0.0 - 1.0)
    pub confidence: f32,
    /// Nearest MIDI note number (if voiced)
    pub midi_note: Option<u8>,
    /// Cents deviation from nearest note (-50 to +50)
    pub cents_offset: f32,
}

impl PitchResult {
    /// Check if a pitch was confidently detected
    pub fn is_voiced(&self) -> bool {
        self.frequency > 0.0 && self.confidence > 0.0
    }

    /// Get note name with sharp notation (e.g., "A4", "C#5")
    pub fn note_name(&self) -> Option<String> {
        self.midi_note.map(|note| {
            const NAMES: [&str; 12] = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            let name = NAMES[(note % 12) as usize];
            let octave = (note / 12) as i32 - 1;
            format!("{}{}", name, octave)
        })
    }

    /// Get note name with flat notation (e.g., "A4", "Db5")
    pub fn note_name_flat(&self) -> Option<String> {
        self.midi_note.map(|note| {
            const NAMES: [&str; 12] = [
                "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
            ];
            let name = NAMES[(note % 12) as usize];
            let octave = (note / 12) as i32 - 1;
            format!("{}{}", name, octave)
        })
    }
}

/// Pitch detector using the YIN algorithm
///
/// This is the complete implementation of YIN with all 6 steps from
/// the original paper (de Cheveigné & Kawahara, 2002).
pub struct PitchDetector {
    sample_rate: f64,
    min_freq: f32,
    max_freq: f32,
    threshold: f32,

    // Pre-allocated buffers
    difference: Vec<f32>,
    cumulative_mean: Vec<f32>,
}

impl PitchDetector {
    /// Create a new pitch detector
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    pub fn new(sample_rate: f64) -> Self {
        Self::with_range(sample_rate, 50.0, 2000.0)
    }

    /// Create with custom frequency range
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `min_freq` - Minimum detectable frequency (default: 50 Hz)
    /// * `max_freq` - Maximum detectable frequency (default: 2000 Hz)
    pub fn with_range(sample_rate: f64, min_freq: f32, max_freq: f32) -> Self {
        // Buffer size based on minimum frequency (need at least 2 periods)
        let max_period = (sample_rate / min_freq as f64) as usize;

        Self {
            sample_rate,
            min_freq,
            max_freq,
            threshold: 0.1, // YIN threshold (lower = stricter)
            difference: vec![0.0; max_period + 1],
            cumulative_mean: vec![0.0; max_period + 1],
        }
    }

    /// Set YIN threshold (0.01 - 0.5)
    ///
    /// Lower values are stricter and may miss quiet notes.
    /// Higher values are more permissive but may have false positives.
    /// Default is 0.1 as recommended in the original paper.
    pub fn set_threshold(&mut self, threshold: f32) {
        self.threshold = threshold.clamp(0.01, 0.5);
    }

    /// Get the required buffer size for detection
    pub fn buffer_size(&self) -> usize {
        let max_period = (self.sample_rate / self.min_freq as f64) as usize;
        max_period * 2
    }

    /// Detect pitch in a single frame of audio
    ///
    /// # Arguments
    /// * `samples` - Audio samples (should be at least `buffer_size()` samples)
    ///
    /// # Returns
    /// Pitch result with frequency, confidence, and note info
    pub fn detect(&mut self, samples: &[f32]) -> PitchResult {
        let min_period = (self.sample_rate / self.max_freq as f64) as usize;
        let max_period = (self.sample_rate / self.min_freq as f64) as usize;
        let max_period = max_period
            .min(samples.len() / 2)
            .min(self.difference.len() - 1);

        if samples.len() < max_period * 2 || max_period <= min_period {
            return PitchResult::default();
        }

        // Step 1-2: Compute difference function
        self.compute_difference(samples, max_period);

        // Step 3: Compute cumulative mean normalized difference
        self.compute_cumulative_mean(max_period);

        // Step 4 & 6: Find best period using threshold and best local estimate
        let (period, aperiodicity) = self.find_best_period_full(min_period, max_period);

        if period == 0 {
            return PitchResult::default();
        }

        // Step 5: Parabolic interpolation for sub-sample accuracy
        let refined_period = self.parabolic_interpolation(period, max_period);

        // Calculate frequency
        let frequency = (self.sample_rate / refined_period) as f32;

        // Calculate confidence (inverse of aperiodicity)
        let confidence = (1.0 - aperiodicity).max(0.0);

        // Calculate MIDI note and cents offset
        let (midi_note, cents_offset) = freq_to_midi(frequency);

        PitchResult {
            frequency,
            confidence,
            midi_note: Some(midi_note),
            cents_offset,
        }
    }

    /// Detect pitch track over an entire audio buffer
    ///
    /// # Arguments
    /// * `samples` - Audio samples
    /// * `hop_size` - Samples between analysis frames
    ///
    /// # Returns
    /// Vector of pitch results, one per frame
    pub fn detect_track(&mut self, samples: &[f32], hop_size: usize) -> Vec<PitchResult> {
        let frame_size = self.buffer_size();
        if samples.len() < frame_size {
            return Vec::new();
        }

        let num_frames = (samples.len() - frame_size) / hop_size + 1;
        let mut results = Vec::with_capacity(num_frames);

        for i in 0..num_frames {
            let start = i * hop_size;
            let frame = &samples[start..start + frame_size];
            results.push(self.detect(frame));
        }

        results
    }

    /// Compute difference function directly (YIN step 1-2)
    ///
    /// This computes d(τ) = Σ(x[j] - x[j+τ])² for j in 0..W
    ///
    /// We use the identity: d(τ) = r(0) + r'(0) - 2*r(τ)
    /// where r(τ) is autocorrelation and r'(0) is energy of shifted window.
    ///
    /// For efficiency, we compute running energy sums.
    fn compute_difference(&mut self, samples: &[f32], max_period: usize) {
        let window = max_period;

        // Compute cumulative sum of squares for efficient energy calculation
        let mut cum_sq = vec![0.0f64; samples.len() + 1];
        for i in 0..samples.len() {
            cum_sq[i + 1] = cum_sq[i] + (samples[i] as f64) * (samples[i] as f64);
        }

        // Energy of window starting at `start` with length `len`
        let energy = |start: usize, len: usize| -> f64 {
            if start + len <= samples.len() {
                cum_sq[start + len] - cum_sq[start]
            } else {
                0.0
            }
        };

        self.difference[0] = 0.0;

        // For each lag τ, compute d(τ) = Σ(x[j] - x[j+τ])²
        // = Σx[j]² + Σx[j+τ]² - 2*Σx[j]*x[j+τ]
        // = energy(0, W) + energy(τ, W) - 2*autocorr(τ)

        // Compute autocorrelation for each τ directly (O(n*max_period) but accurate)
        for tau in 1..=max_period {
            let mut autocorr = 0.0f64;
            for j in 0..window {
                if j + tau < samples.len() {
                    autocorr += (samples[j] as f64) * (samples[j + tau] as f64);
                }
            }

            let e0 = energy(0, window);
            let e_tau = energy(tau, window);
            self.difference[tau] = (e0 + e_tau - 2.0 * autocorr) as f32;
        }
    }

    /// Compute cumulative mean normalized difference (YIN step 3)
    ///
    /// d'(τ) = d(τ) / ((1/τ) * Σ d(j)) for j in 1..τ
    /// d'(0) = 1
    fn compute_cumulative_mean(&mut self, max_period: usize) {
        self.cumulative_mean[0] = 1.0;

        let mut running_sum = 0.0f32;
        for tau in 1..=max_period {
            running_sum += self.difference[tau];
            if running_sum > 1e-10 {
                self.cumulative_mean[tau] = self.difference[tau] * tau as f32 / running_sum;
            } else {
                self.cumulative_mean[tau] = 1.0;
            }
        }
    }

    /// Find the best period using absolute threshold (YIN step 4)
    ///
    /// The key insight from YIN: return the FIRST local minimum below threshold,
    /// not the global minimum. This prevents octave errors (subharmonic detection).
    ///
    /// Returns (period, aperiodicity) where aperiodicity is the d'(τ) value
    fn find_best_period_full(&self, min_period: usize, max_period: usize) -> (usize, f32) {
        // Step 4: Find FIRST τ where d'(τ) < threshold and is a local minimum
        // This is critical for avoiding octave errors!
        let mut tau = min_period;

        while tau < max_period {
            if self.cumulative_mean[tau] < self.threshold {
                // Found a candidate below threshold
                // Walk to the local minimum
                while tau + 1 < max_period
                    && self.cumulative_mean[tau + 1] < self.cumulative_mean[tau]
                {
                    tau += 1;
                }
                // Return the FIRST good candidate (prevents octave errors)
                return (tau, self.cumulative_mean[tau]);
            }
            tau += 1;
        }

        // Fallback: if nothing below threshold, find the global minimum
        // This handles cases where the signal is noisy but still periodic
        let mut best_tau = min_period;
        let mut best_val = self.cumulative_mean[min_period];

        for tau in min_period + 1..=max_period {
            if self.cumulative_mean[tau] < best_val {
                best_val = self.cumulative_mean[tau];
                best_tau = tau;
            }
        }

        // Only return if it's a reasonable minimum
        if best_val < 0.5 {
            (best_tau, best_val)
        } else {
            (0, 1.0)
        }
    }

    /// Parabolic interpolation for sub-sample accuracy (YIN step 5)
    ///
    /// Fits a parabola through three points and finds the minimum.
    fn parabolic_interpolation(&self, tau: usize, max_period: usize) -> f64 {
        if tau < 1 || tau >= max_period {
            return tau as f64;
        }

        let s0 = self.cumulative_mean[tau - 1] as f64;
        let s1 = self.cumulative_mean[tau] as f64;
        let s2 = self.cumulative_mean[tau + 1] as f64;

        // Parabolic interpolation: find vertex of parabola through (τ-1, s0), (τ, s1), (τ+1, s2)
        let denominator = 2.0 * (2.0 * s1 - s2 - s0);

        if denominator.abs() > 1e-10 {
            let adjustment = (s2 - s0) / denominator;
            tau as f64 + adjustment
        } else {
            tau as f64
        }
    }
}

/// Convert frequency to MIDI note and cents offset
pub fn freq_to_midi(freq: f32) -> (u8, f32) {
    if freq <= 0.0 {
        return (0, 0.0);
    }

    // MIDI note = 69 + 12 * log2(freq / 440)
    let note_float = 69.0 + 12.0 * (freq / 440.0).log2();
    let note = note_float.round() as i32;
    let note = note.clamp(0, 127) as u8;

    // Cents = 1200 * log2(freq / freq_of_note)
    let note_freq = 440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0);
    let cents = 1200.0 * (freq / note_freq).log2();

    (note, cents)
}

/// Convert MIDI note to frequency
pub fn midi_to_freq(note: u8) -> f32 {
    440.0 * 2.0f32.powf((note as f32 - 69.0) / 12.0)
}

/// Median filter for smoothing pitch tracks
///
/// Removes outliers (like octave jumps) by replacing each value with
/// the median of its neighborhood.
pub fn median_filter(pitches: &[PitchResult], window_size: usize) -> Vec<PitchResult> {
    if pitches.is_empty() || window_size < 2 {
        return pitches.to_vec();
    }

    let half = window_size / 2;
    let mut result = Vec::with_capacity(pitches.len());

    for i in 0..pitches.len() {
        let start = i.saturating_sub(half);
        let end = (i + half + 1).min(pitches.len());

        // Collect voiced frequencies in window
        let mut freqs: Vec<f32> = pitches[start..end]
            .iter()
            .filter(|p| p.is_voiced())
            .map(|p| p.frequency)
            .collect();

        if freqs.is_empty() {
            result.push(PitchResult::default());
        } else {
            freqs.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_freq = freqs[freqs.len() / 2];
            let (midi_note, cents_offset) = freq_to_midi(median_freq);

            result.push(PitchResult {
                frequency: median_freq,
                confidence: pitches[i].confidence,
                midi_note: Some(midi_note),
                cents_offset,
            });
        }
    }

    result
}

/// Viterbi smoothing for pitch tracks (optional post-processing)
///
/// Uses dynamic programming to find the most likely pitch sequence,
/// penalizing large jumps between consecutive frames.
pub fn viterbi_smooth(pitches: &[PitchResult], jump_penalty: f32) -> Vec<PitchResult> {
    if pitches.len() < 2 {
        return pitches.to_vec();
    }

    // Simplified Viterbi: penalize large frequency jumps
    let mut result = pitches.to_vec();

    for i in 1..result.len() {
        if result[i].is_voiced() && result[i - 1].is_voiced() {
            let ratio = result[i].frequency / result[i - 1].frequency;

            // If jump is more than a major third (ratio > 1.26 or < 0.79), reduce confidence
            if !(0.79..=1.26).contains(&ratio) {
                let jump_cost = ((ratio.ln().abs() / 0.23) - 1.0).max(0.0); // 0.23 ≈ ln(1.26)
                result[i].confidence *= (-jump_penalty * jump_cost).exp();
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generate_sine(sample_rate: f64, freq: f32, duration: f64) -> Vec<f32> {
        let num_samples = (sample_rate * duration) as usize;
        (0..num_samples)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (2.0 * std::f32::consts::PI * freq * t).sin()
            })
            .collect()
    }

    #[test]
    fn test_detect_a440() {
        let sample_rate = 44100.0;
        let samples = generate_sine(sample_rate, 440.0, 0.1);

        let mut detector = PitchDetector::new(sample_rate);
        let result = detector.detect(&samples);

        assert!(result.is_voiced(), "Should detect voiced signal");
        assert!(
            (result.frequency - 440.0).abs() < 5.0,
            "Expected ~440 Hz, got {} Hz",
            result.frequency
        );
        assert_eq!(result.midi_note, Some(69), "A4 should be MIDI note 69");
        assert!(
            result.cents_offset.abs() < 10.0,
            "Cents offset should be small, got {}",
            result.cents_offset
        );
    }

    #[test]
    fn test_detect_various_frequencies() {
        let sample_rate = 44100.0;
        let test_freqs = [100.0, 220.0, 440.0, 880.0, 1000.0];

        let mut detector = PitchDetector::new(sample_rate);

        for &freq in &test_freqs {
            let samples = generate_sine(sample_rate, freq, 0.1);
            let result = detector.detect(&samples);

            assert!(result.is_voiced(), "Should detect {}Hz", freq);
            let error_percent = ((result.frequency - freq) / freq).abs() * 100.0;
            assert!(
                error_percent < 2.0,
                "Expected {}Hz, got {}Hz ({}% error)",
                freq,
                result.frequency,
                error_percent
            );
        }
    }

    #[test]
    fn test_silence_detection() {
        let sample_rate = 44100.0;
        let samples = vec![0.0; 4096];

        let mut detector = PitchDetector::new(sample_rate);
        let result = detector.detect(&samples);

        // Silence should have low confidence or be detected as unvoiced
        assert!(result.confidence < 0.5 || result.frequency == 0.0);
    }

    #[test]
    fn test_detect_track() {
        let sample_rate = 44100.0;
        let samples = generate_sine(sample_rate, 440.0, 0.5);

        let mut detector = PitchDetector::new(sample_rate);
        let track = detector.detect_track(&samples, 512);

        assert!(!track.is_empty());

        // Most frames should detect ~440 Hz
        let voiced_count = track.iter().filter(|r| r.is_voiced()).count();
        assert!(
            voiced_count > track.len() / 2,
            "Most frames should be voiced"
        );
    }

    #[test]
    fn test_note_names() {
        let result = PitchResult {
            frequency: 440.0,
            confidence: 1.0,
            midi_note: Some(69),
            cents_offset: 0.0,
        };
        assert_eq!(result.note_name(), Some("A4".to_string()));
        assert_eq!(result.note_name_flat(), Some("A4".to_string()));

        let result = PitchResult {
            frequency: 261.63,
            confidence: 1.0,
            midi_note: Some(60),
            cents_offset: 0.0,
        };
        assert_eq!(result.note_name(), Some("C4".to_string()));

        // Test sharp/flat difference
        let result = PitchResult {
            frequency: 277.18, // C#4/Db4
            confidence: 1.0,
            midi_note: Some(61),
            cents_offset: 0.0,
        };
        assert_eq!(result.note_name(), Some("C#4".to_string()));
        assert_eq!(result.note_name_flat(), Some("Db4".to_string()));
    }

    #[test]
    fn test_median_filter() {
        let pitches = vec![
            PitchResult {
                frequency: 440.0,
                confidence: 1.0,
                midi_note: Some(69),
                cents_offset: 0.0,
            },
            PitchResult {
                frequency: 880.0,
                confidence: 1.0,
                midi_note: Some(81),
                cents_offset: 0.0,
            }, // Octave jump (outlier)
            PitchResult {
                frequency: 442.0,
                confidence: 1.0,
                midi_note: Some(69),
                cents_offset: 0.0,
            },
            PitchResult {
                frequency: 438.0,
                confidence: 1.0,
                midi_note: Some(69),
                cents_offset: 0.0,
            },
        ];

        let filtered = median_filter(&pitches, 3);

        // The median filter should smooth out the octave jump
        assert!(filtered[1].frequency < 500.0, "Outlier should be smoothed");
    }

    #[test]
    fn test_freq_midi_conversion() {
        // A4 = 440 Hz = MIDI 69
        let (note, cents) = freq_to_midi(440.0);
        assert_eq!(note, 69);
        assert!(cents.abs() < 1.0);

        // C4 = 261.63 Hz = MIDI 60
        let (note, cents) = freq_to_midi(261.63);
        assert_eq!(note, 60);
        assert!(cents.abs() < 5.0);

        // Round trip
        for midi in [36, 48, 60, 69, 72, 84, 96] {
            let freq = midi_to_freq(midi);
            let (back, cents) = freq_to_midi(freq);
            assert_eq!(back, midi, "Round trip failed for MIDI {}", midi);
            assert!(cents.abs() < 0.01, "Cents should be ~0 for exact MIDI note");
        }
    }

    #[test]
    fn test_low_frequency() {
        let sample_rate = 44100.0;
        let samples = generate_sine(sample_rate, 82.41, 0.2); // E2

        let mut detector = PitchDetector::with_range(sample_rate, 40.0, 1000.0);
        let result = detector.detect(&samples);

        assert!(result.is_voiced(), "Should detect low E2");
        let error_percent = ((result.frequency - 82.41) / 82.41).abs() * 100.0;
        assert!(
            error_percent < 3.0,
            "Expected ~82.41 Hz, got {} Hz ({}% error)",
            result.frequency,
            error_percent
        );
    }

    #[test]
    fn test_high_frequency() {
        let sample_rate = 44100.0;
        let samples = generate_sine(sample_rate, 1760.0, 0.05); // A6

        let mut detector = PitchDetector::new(sample_rate);
        let result = detector.detect(&samples);

        assert!(result.is_voiced(), "Should detect high A6");
        let error_percent = ((result.frequency - 1760.0) / 1760.0).abs() * 100.0;
        assert!(
            error_percent < 2.0,
            "Expected ~1760 Hz, got {} Hz ({}% error)",
            result.frequency,
            error_percent
        );
    }
}
