//! Error types for tutti-synth.

use thiserror::Error;

/// Result type alias for tutti-synth operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in tutti-synth.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error (file operations).
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid configuration parameter.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// SoundFont loading or playback error.
    #[error("SoundFont error: {0}")]
    SoundFont(String),
}
