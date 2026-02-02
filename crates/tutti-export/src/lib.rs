//! # Tutti Export
//!
//! Audio export utilities for the Tutti audio engine.
//!
//! This crate provides low-level export functionality:
//! - **Format encoding**: Export to WAV, FLAC
//! - **DSP utilities**: Resampling, dithering, loudness metering
//!
//! ## Note
//!
//! This crate is typically not used directly. Instead, use the integrated
//! export API from the main `tutti` crate:
//!
//! ```ignore
//! use tutti::prelude::*;
//!
//! let engine = TuttiEngine::builder().build()?;
//! // ... build your graph ...
//!
//! // Export directly from engine
//! engine.export()
//!     .duration_seconds(10.0)
//!     .to_file("output.wav")?;
//! ```
//!
//! ## Feature Flags
//!
//! - `wav` (default): WAV export via hound (pure Rust)
//! - `flac` (default): FLAC export via flacenc (pure Rust)
//! - `butler`: Butler thread integration for async disk I/O

// Core modules
pub mod error;
pub mod export_builder;
mod options;

// Advanced APIs
pub mod dsp;
pub mod format;

// Butler async export (requires tutti-sampler)
#[cfg(feature = "butler")]
pub mod butler_export;

// Re-exports
pub use error::{ExportError, Result};
pub use export_builder::ExportBuilder;
pub use options::{
    AudioFormat, BitDepth, DitherType, ExportOptions, ExportRange, FlacOptions, NormalizationMode,
    SampleRateTarget,
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
#[allow(unused_variables)]
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
