//! Error types for tutti-synth.

use thiserror::Error;

/// Result type alias for tutti-synth operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur in tutti-synth.
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error (file operations, soundfont feature only).
    #[cfg(feature = "soundfont")]
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Invalid configuration parameter.
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// SoundFont loading or playback error.
    #[cfg(feature = "soundfont")]
    #[error("SoundFont error: {0}")]
    SoundFont(String),
}

// Allow converting synth errors to core errors
impl From<Error> for tutti_core::Error {
    fn from(e: Error) -> Self {
        tutti_core::Error::Synth(e.to_string())
    }
}
