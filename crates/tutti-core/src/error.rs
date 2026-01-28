//! Error types for tutti-core.

use thiserror::Error;

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

    #[error("Audio device not available")]
    DeviceNotAvailable(#[from] cpal::DefaultStreamConfigError),

    #[error("Failed to build audio stream")]
    BuildStream(#[from] cpal::BuildStreamError),

    #[error("Failed to play audio stream")]
    PlayStream(#[from] cpal::PlayStreamError),

    #[error("Failed to enumerate devices")]
    DevicesError(#[from] cpal::DevicesError),

    #[error("Failed to get device name")]
    DeviceNameError(#[from] cpal::DeviceNameError),

    #[error("Lock poisoned")]
    LockPoisoned,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Not implemented: {0}")]
    NotImplemented(String),
}

/// Result type alias.
pub type Result<T> = std::result::Result<T, Error>;
