//! Audio format encoders
//!
//! Each encoder is feature-gated:
//! - `wav`: WAV via hound (pure Rust)
//! - `flac`: FLAC via flacenc (pure Rust)

#[cfg(feature = "wav")]
pub mod wav;

#[cfg(feature = "flac")]
pub mod flac;
