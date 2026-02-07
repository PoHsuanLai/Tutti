//! Error types for tutti-dsp

use thiserror::Error;

/// Error type for DSP operations
#[derive(Debug, Clone, Error)]
pub enum Error {
    /// Invalid channel count for the operation
    #[error("Invalid channel count: {0}")]
    InvalidChannelCount(String),

    /// Invalid speaker configuration
    #[error("Invalid speaker configuration: {0}")]
    InvalidSpeakerConfig(String),

    /// Invalid parameter value
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// VBAP spatial audio error
    #[cfg(feature = "spatial")]
    #[error("VBAP error: {0}")]
    VBAPError(String),
}

// Convert VBAP errors to our error type
#[cfg(feature = "spatial")]
impl From<vbap::VBAPError> for Error {
    fn from(err: vbap::VBAPError) -> Self {
        Error::VBAPError(err.to_string())
    }
}

/// Result type for DSP operations
pub type Result<T> = core::result::Result<T, Error>;
