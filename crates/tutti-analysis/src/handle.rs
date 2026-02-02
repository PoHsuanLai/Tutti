//! Analysis handle for convenient API access

use crate::{
    CorrelationMeter, DetectionMethod, PitchDetector, PitchResult, StereoAnalysis,
    StereoWaveformSummary, Transient, TransientDetector, WaveformSummary,
};

#[cfg(feature = "cache")]
use crate::ThumbnailCache;

#[cfg(feature = "cache")]
use std::sync::{Arc, Mutex};

/// Handle for audio analysis tools
///
/// Provides convenient methods for transient detection, pitch detection,
/// stereo correlation, and waveform thumbnails.
pub struct AnalysisHandle {
    sample_rate: f64,
    #[cfg(feature = "cache")]
    thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
}

impl AnalysisHandle {
    /// Create a new analysis handle
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate,
            #[cfg(feature = "cache")]
            thumbnail_cache: Arc::new(Mutex::new(ThumbnailCache::new(1024))),
        }
    }

    /// Detect transients using default method (spectral flux)
    pub fn detect_transients(&self, samples: &[f32]) -> Vec<Transient> {
        let mut detector = TransientDetector::new(self.sample_rate);
        detector.detect(samples)
    }

    /// Detect transients with custom detection method
    pub fn detect_transients_with_method(
        &self,
        samples: &[f32],
        method: DetectionMethod,
    ) -> Vec<Transient> {
        let mut detector = TransientDetector::new(self.sample_rate);
        detector.set_method(method);
        detector.detect(samples)
    }

    /// Detect pitch (fundamental frequency)
    ///
    /// Returns result with frequency and confidence. Check confidence
    /// to determine if the detection is reliable.
    pub fn detect_pitch(&self, samples: &[f32]) -> PitchResult {
        let mut detector = PitchDetector::new(self.sample_rate);
        detector.detect(samples)
    }

    /// Detect pitch with minimum confidence threshold
    ///
    /// Only returns results with confidence >= min_confidence.
    pub fn detect_pitch_with_confidence(
        &self,
        samples: &[f32],
        min_confidence: f32,
    ) -> Option<PitchResult> {
        let result = self.detect_pitch(samples);
        if result.confidence >= min_confidence {
            Some(result)
        } else {
            None
        }
    }

    /// Analyze stereo correlation, width, and balance
    pub fn analyze_stereo(&self, left: &[f32], right: &[f32]) -> StereoAnalysis {
        let mut meter = CorrelationMeter::new(self.sample_rate);
        meter.process(left, right)
    }

    /// Generate waveform summary for visualization (mono)
    pub fn waveform_summary(&self, samples: &[f32], samples_per_block: usize) -> WaveformSummary {
        crate::waveform::compute_summary(samples, 1, samples_per_block)
    }

    /// Generate stereo waveform summary from interleaved samples
    ///
    /// Input should be interleaved stereo: [L0, R0, L1, R1, ...]
    pub fn stereo_waveform_summary(
        &self,
        interleaved_samples: &[f32],
        samples_per_block: usize,
    ) -> StereoWaveformSummary {
        crate::waveform::compute_stereo_summary(interleaved_samples, samples_per_block)
    }

    /// Get or compute multi-resolution waveform summary with caching
    ///
    /// This uses the thumbnail cache to avoid recomputing summaries.
    /// Creates 8 zoom levels starting at 512 samples/block.
    #[cfg(feature = "cache")]
    pub fn cached_multi_resolution_summary(
        &self,
        audio_id: u64,
        samples: &[f32],
    ) -> crate::MultiResolutionSummary {
        let mut cache = self.thumbnail_cache.lock().unwrap();
        cache
            .get_or_compute(audio_id, || {
                crate::MultiResolutionSummary::from_samples(samples, 1, 512, 8)
            })
            .clone()
    }

    /// Clear waveform thumbnail cache
    #[cfg(feature = "cache")]
    pub fn clear_cache(&self) {
        self.thumbnail_cache.lock().unwrap().clear();
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}
