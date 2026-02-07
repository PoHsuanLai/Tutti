//! Analysis handle for convenient API access

use crate::live::LiveAnalysisState;
use crate::ThumbnailCache;
use crate::{
    CorrelationMeter, DetectionMethod, PitchDetector, PitchResult, StereoAnalysis,
    StereoWaveformSummary, Transient, TransientDetector, WaveformSummary,
};
use std::sync::{Arc, Mutex};

/// Handle for audio analysis tools
///
/// Provides convenient methods for transient detection, pitch detection,
/// stereo correlation, and waveform thumbnails.
///
/// When live analysis is enabled (via `TuttiEngine::enable_live_analysis()`),
/// the `live_*()` methods return results computed from the running audio graph.
pub struct AnalysisHandle {
    sample_rate: f64,
    live: Option<Arc<LiveAnalysisState>>,
    thumbnail_cache: Arc<Mutex<ThumbnailCache>>,
}

impl AnalysisHandle {
    /// Create a new analysis handle (offline only)
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sample_rate,
            live: None,
            thumbnail_cache: Arc::new(Mutex::new(ThumbnailCache::new(1024))),
        }
    }

    /// Create an analysis handle with live state attached
    pub fn with_live(sample_rate: f64, live: Arc<LiveAnalysisState>) -> Self {
        Self {
            sample_rate,
            live: Some(live),
            thumbnail_cache: Arc::new(Mutex::new(ThumbnailCache::new(1024))),
        }
    }

    /// Whether live analysis is active.
    pub fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// Get the latest pitch detection from the live audio stream.
    ///
    /// Returns `PitchResult::default()` if live analysis is not enabled.
    pub fn live_pitch(&self) -> Arc<PitchResult> {
        match &self.live {
            Some(state) => state.pitch.load_full(),
            None => Arc::new(PitchResult::default()),
        }
    }

    /// Get recent transient detections from the live audio stream.
    ///
    /// Returns an empty vec if live analysis is not enabled.
    pub fn live_transients(&self) -> Arc<Vec<Transient>> {
        match &self.live {
            Some(state) => state.transients.load_full(),
            None => Arc::new(Vec::new()),
        }
    }

    /// Get the rolling waveform summary from the live audio stream.
    ///
    /// Returns the last ~2 seconds of waveform blocks for visualization.
    pub fn live_waveform(&self) -> Arc<WaveformSummary> {
        match &self.live {
            Some(state) => state.waveform.load_full(),
            None => Arc::new(WaveformSummary::new(512)),
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
    pub fn clear_cache(&self) {
        self.thumbnail_cache.lock().unwrap().clear();
    }

    /// Get sample rate
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }
}
