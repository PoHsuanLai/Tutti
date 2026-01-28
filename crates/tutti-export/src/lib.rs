//! # Tutti Export
//!
//! Offline audio export and rendering for the Tutti audio engine.
//!
//! This crate provides:
//! - **Offline rendering**: Render audio graphs to memory buffers
//! - **Format encoding**: Export to WAV, FLAC
//! - **DSP utilities**: Resampling, dithering, loudness metering
//!
//! ## Architecture
//!
//! The export system is instruction-driven and framework-agnostic:
//! - Receives pre-computed sample positions (no IR dependency)
//! - Works with synth/effect indices, not high-level objects
//! - Frontend converts beats to samples before calling renderer
//!
//! ## Example
//!
//! ```ignore
//! use tutti_export::{OfflineRenderer, RenderJob, ExportOptions, AudioFormat};
//!
//! // Create renderer
//! let renderer = OfflineRenderer::new(44100);
//!
//! // Define what to render
//! let job = RenderJob::new(44100, 44100 * 10) // 10 seconds
//!     .with_track(RenderTrack::new(0)
//!         .with_note(RenderNote {
//!             synth_index: 0,
//!             midi_note: 60,
//!             velocity: 100,
//!             start_sample: 0,
//!             duration_samples: 44100,
//!             params: None,
//!         }));
//!
//! // Render to buffer
//! let result = renderer.render(job, None)?;
//!
//! // Export to file
//! let options = ExportOptions::default();
//! tutti_export::export_wav("output.wav", &result.left, &result.right, &options)?;
//! ```
//!
//! ## Feature Flags
//!
//! - `wav` (default): WAV export via hound (pure Rust)
//! - `flac` (default): FLAC export via flacenc (pure Rust)
//! - `butler`: Butler thread integration for async disk I/O

// Core modules
pub mod error;
mod options;
mod renderer;
mod types;

// Advanced APIs
pub mod dsp;
pub mod format;

// Butler async export (requires tutti-sampler)
#[cfg(feature = "butler")]
pub mod butler_export;

// Re-exports
pub use error::{ExportError, Result};
pub use options::{
    AudioFormat, BitDepth, DitherType, ExportOptions, ExportRange, FlacOptions, NormalizationMode,
    SampleRateTarget,
};
pub use renderer::{OfflineRenderer, RenderResult};
pub use types::{
    RenderAudioClip, RenderJob, RenderMaster, RenderNote, RenderPatternTrigger, RenderTrack,
};

// Butler export (requires tutti-sampler)
#[cfg(feature = "butler")]
pub use butler_export::ButlerExporter;

// Format-specific exports
#[cfg(feature = "wav")]
pub use format::wav::export_wav;

#[cfg(feature = "flac")]
pub use format::flac::export_flac;

/// Export audio to a file with automatic format detection
///
/// The format is determined by the file extension:
/// - `.wav` -> WAV
/// - `.flac` -> FLAC
pub fn export_to_file(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
) -> Result<()> {
    let path_lower = path.to_lowercase();

    if path_lower.ends_with(".wav") {
        #[cfg(feature = "wav")]
        return format::wav::export_wav(path, left, right, options);
        #[cfg(not(feature = "wav"))]
        return Err(ExportError::UnsupportedFormat(
            "WAV support not enabled".into(),
        ));
    }

    if path_lower.ends_with(".flac") {
        #[cfg(feature = "flac")]
        return format::flac::export_flac(path, left, right, options);
        #[cfg(not(feature = "flac"))]
        return Err(ExportError::UnsupportedFormat(
            "FLAC support not enabled".into(),
        ));
    }

    Err(ExportError::UnsupportedFormat(format!(
        "Unknown or unsupported file extension: {}. Supported: .wav, .flac",
        path
    )))
}
