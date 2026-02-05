//! Error types for tutti-core.

use crate::compat::String;
use thiserror::Error;

#[cfg(feature = "std")]
#[allow(unused_extern_crates)]
extern crate std;

/// Error type for tutti-core operations.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid config: {0}")]
    InvalidConfig(String),

    #[error("Invalid tempo: {0}. Must be between 20.0 and 999.0 BPM")]
    InvalidTempo(f32),

    #[error("Invalid beat position: {0}. Must be non-negative")]
    InvalidBeat(f64),

    #[error("Invalid loop range: start={start}, end={end}")]
    InvalidLoopRange { start: f64, end: f64 },

    #[error("Invalid time signature: {numerator}/{denominator}")]
    InvalidTimeSignature { numerator: u32, denominator: u32 },

    #[error("Invalid device: {0}")]
    InvalidDevice(String),

    #[cfg(feature = "std")]
    #[error("Audio device not available")]
    DeviceNotAvailable(#[from] cpal::DefaultStreamConfigError),

    #[cfg(feature = "std")]
    #[error("Failed to build audio stream")]
    BuildStream(#[from] cpal::BuildStreamError),

    #[cfg(feature = "std")]
    #[error("Failed to play audio stream")]
    PlayStream(#[from] cpal::PlayStreamError),

    #[cfg(feature = "std")]
    #[error("Failed to enumerate devices")]
    DevicesError(#[from] cpal::DevicesError),

    #[cfg(feature = "std")]
    #[error("Failed to get device name")]
    DeviceNameError(#[from] cpal::DeviceNameError),

    #[error("Lock poisoned")]
    LockPoisoned,

    #[cfg(feature = "std")]
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("LUFS measurement not available (already in use or not initialized)")]
    LufsNotReady,

    #[error("Synth error: {0}")]
    Synth(String),
}

/// Result type alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors from node registry operations.
#[derive(Error, Debug)]
pub enum NodeRegistryError {
    #[error("Unknown node type: {0}")]
    UnknownNodeType(String),

    #[error("Missing parameter: {0}")]
    MissingParameter(String),

    #[error("Invalid parameter '{0}': {1}")]
    InvalidParameter(String, String),

    #[error("Construction failed: {0}")]
    ConstructionFailed(String),

    #[error("Neural: {0}")]
    Neural(String),

    #[error("Plugin: {0}")]
    Plugin(String),

    #[error("Audio file: {0}")]
    AudioFile(String),
}
