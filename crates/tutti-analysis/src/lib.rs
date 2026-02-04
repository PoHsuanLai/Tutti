//! # Tutti Analysis
//!
//! Audio analysis tools for DAW applications.
//!
//! This crate provides efficient algorithms for:
//! - **Waveform thumbnails**: Multi-resolution min/max/RMS summaries for visualization
//! - **Transient detection**: Onset/beat detection using spectral flux and other methods
//! - **Pitch detection**: Monophonic pitch tracking using the YIN algorithm
//! - **Stereo correlation**: Phase correlation, stereo width, and balance analysis
//!
//! All functions operate on raw `&[f32]` sample buffers - no framework dependencies.
//!
//! ## TODO: SIMD Optimization
//!
//! The following hot paths could benefit from SIMD (via `wide` crate):
//!
//! - `correlation::analyze_stereo()` - 5 parallel accumulators (L², R², L*R, M², S²)
//! - `pitch::compute_difference()` - autocorrelation dot product inner loop
//! - `waveform::compute_summary()` - min/max/sum_sq accumulation
//! - `waveform::compute_stereo_summary()` - dual-channel min/max/rms
//!
//! ## Example
//!
//! ```rust
//! use tutti_analysis::{
//!     waveform::compute_summary,
//!     transient::TransientDetector,
//!     pitch::PitchDetector,
//!     correlation::CorrelationMeter,
//! };
//!
//! let samples: Vec<f32> = vec![0.0; 44100]; // 1 second of audio
//! let sample_rate = 44100.0;
//!
//! // Waveform thumbnail
//! let summary = compute_summary(&samples, 1, 512);
//!
//! // Transient detection
//! let mut detector = TransientDetector::new(sample_rate);
//! let transients = detector.detect(&samples);
//!
//! // Pitch detection (needs at least buffer_size() samples)
//! let mut pitch_detector = PitchDetector::new(sample_rate);
//! let pitch = pitch_detector.detect(&samples);
//!
//! // Stereo correlation (for stereo audio)
//! let left = &samples[..];
//! let right = &samples[..];
//! let mut meter = CorrelationMeter::new(sample_rate);
//! let analysis = meter.process(left, right);
//! ```

pub mod correlation;
pub mod pitch;
pub mod transient;
pub mod waveform;

#[cfg(feature = "cache")]
pub mod cache;

#[cfg(feature = "live")]
pub mod live;

// Fluent API handle
mod handle;

// Re-export main types at crate root for convenience
pub use correlation::{CorrelationMeter, StereoAnalysis};
pub use pitch::{
    freq_to_midi, median_filter, midi_to_freq, viterbi_smooth, PitchDetector, PitchResult,
};
pub use transient::{DetectionMethod, Transient, TransientDetector};
pub use waveform::{MultiResolutionSummary, StereoWaveformSummary, WaveformBlock, WaveformSummary};

#[cfg(feature = "cache")]
pub use cache::ThumbnailCache;

#[cfg(feature = "live")]
pub use live::LiveAnalysisState;

pub use handle::AnalysisHandle;
