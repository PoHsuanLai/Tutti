//! # Tutti Export
//!
//! Offline audio export for the Tutti audio engine.
//!
//! ## When to Use
//!
//! Use this crate for **offline rendering** with full DSP processing:
//! - Bounce/export a mix to WAV or FLAC
//! - Sample rate conversion, bit depth reduction
//! - LUFS normalization, dithering
//!
//! For **real-time recording** (capturing live audio input), use
//! `tutti-sampler`'s recording API instead.
//!
//! ## Usage
//!
//! Via the main `tutti` crate:
//!
//! ```ignore
//! engine.export()
//!     .duration_seconds(10.0)
//!     .normalize(NormalizationMode::lufs(-14.0))
//!     .to_file("output.flac")?;
//!
//! // With progress callback
//! engine.export()
//!     .duration_seconds(3600.0)
//!     .to_file_with_progress("output.wav", |p| {
//!         println!("{:?}: {:.0}%", p.phase, p.progress * 100.0);
//!     })?;
//! ```
//!
//! Or standalone:
//!
//! ```ignore
//! use tutti_export::{export_to_file, ExportOptions};
//!
//! export_to_file("output.wav", &left, &right, &ExportOptions::default())?;
//! ```
//!
//! ## Features
//!
//! - `wav` (default): WAV encoding
//! - `flac` (default): FLAC encoding

// Error types
mod error;
pub use error::{ExportError, Result};

// Export builder
mod export_builder;
pub use export_builder::{ExportBuilder, ExportPhase, ExportProgress};

// Options
mod options;
pub use options::{
    AudioFormat, BitDepth, DitherType, ExportOptions, FlacOptions, NormalizationMode,
};

// DSP utilities
pub(crate) mod dsp;
pub use dsp::ResampleQuality;

// Format encoders
pub(crate) mod format;

/// Export audio to a file (format detected from extension).
#[allow(unused_variables)]
pub fn export_to_file(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
) -> Result<()> {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    match ext.as_str() {
        #[cfg(feature = "wav")]
        "wav" => format::wav::export_wav(path, left, right, options),
        #[cfg(not(feature = "wav"))]
        "wav" => Err(ExportError::UnsupportedFormat("WAV not enabled".into())),

        #[cfg(feature = "flac")]
        "flac" => format::flac::export_flac(path, left, right, options),
        #[cfg(not(feature = "flac"))]
        "flac" => Err(ExportError::UnsupportedFormat("FLAC not enabled".into())),

        _ => Err(ExportError::UnsupportedFormat(format!(
            "Unknown extension: .{}",
            ext
        ))),
    }
}

/// Export audio to a file with progress callback.
#[allow(unused_variables)]
pub fn export_to_file_with_progress(
    path: &str,
    left: &[f32],
    right: &[f32],
    options: &ExportOptions,
    on_progress: impl Fn(ExportProgress),
) -> Result<()> {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();

    match ext.as_str() {
        #[cfg(feature = "wav")]
        "wav" => format::wav::export_wav_with_progress(path, left, right, options, on_progress),
        #[cfg(not(feature = "wav"))]
        "wav" => Err(ExportError::UnsupportedFormat("WAV not enabled".into())),

        #[cfg(feature = "flac")]
        "flac" => format::flac::export_flac_with_progress(path, left, right, options, on_progress),
        #[cfg(not(feature = "flac"))]
        "flac" => Err(ExportError::UnsupportedFormat("FLAC not enabled".into())),

        _ => Err(ExportError::UnsupportedFormat(format!(
            "Unknown extension: .{}",
            ext
        ))),
    }
}
