//! Error types for tutti-export

use std::io;
use thiserror::Error;

/// Export error type
#[derive(Error, Debug)]
pub enum ExportError {
    /// I/O error during file operations
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Unsupported format or feature not enabled
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    /// Invalid export options
    #[error("Invalid options: {0}")]
    InvalidOptions(String),

    /// Encoding error
    #[error("Encoding error: {0}")]
    Encoding(String),

    /// Rendering error
    #[error("Render error: {0}")]
    Render(String),

    /// Resampling error
    #[error("Resampling error: {0}")]
    Resample(String),

    /// Invalid audio data
    #[error("Invalid audio data: {0}")]
    InvalidData(String),
}

/// Result type for export operations
pub type Result<T> = std::result::Result<T, ExportError>;

// From trait implementations for external library errors at API boundary
// These convert complex external error types to simple strings for user-facing messages

#[cfg(feature = "wav")]
impl From<hound::Error> for ExportError {
    fn from(e: hound::Error) -> Self {
        ExportError::Io(io::Error::other(e))
    }
}

impl From<rubato::ResamplerConstructionError> for ExportError {
    fn from(e: rubato::ResamplerConstructionError) -> Self {
        ExportError::Resample(e.to_string())
    }
}

impl From<rubato::ResampleError> for ExportError {
    fn from(e: rubato::ResampleError) -> Self {
        ExportError::Resample(e.to_string())
    }
}
