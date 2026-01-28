//! Error types.

use thiserror::Error;

/// Error type.
#[derive(Error, Debug)]
pub enum Error {
    /// IO error.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Sample not found.
    #[error("Sample not found: {0}")]
    SampleNotFound(String),

    /// Recording error.
    #[error("Recording error: {0}")]
    Recording(String),

    /// Butler error.
    #[error("Butler error: {0}")]
    Butler(String),

    /// Audio input error.
    #[error("Audio input error: {0}")]
    AudioInput(String),

    /// SoundFont error.
    #[error("SoundFont error: {0}")]
    SoundFont(String),

    /// Time stretch error.
    #[error("Time stretch error: {0}")]
    TimeStretch(String),

    /// Failed to enumerate devices.
    #[error("Failed to enumerate audio devices")]
    DevicesError(#[from] cpal::DevicesError),

    /// Failed to get device config.
    #[error("Failed to get audio device config")]
    DeviceConfigError(#[from] cpal::DefaultStreamConfigError),

    /// Failed to build stream.
    #[error("Failed to build audio stream")]
    BuildStreamError(#[from] cpal::BuildStreamError),

    /// Failed to play stream.
    #[error("Failed to play audio stream")]
    PlayStreamError(#[from] cpal::PlayStreamError),

    /// Device not found.
    #[error("Audio device not found: {0}")]
    DeviceNotFound(String),

    /// Hound error.
    #[error("Hound error: {0}")]
    HoundError(#[from] hound::Error),
}

/// Result type.
pub type Result<T> = std::result::Result<T, Error>;
