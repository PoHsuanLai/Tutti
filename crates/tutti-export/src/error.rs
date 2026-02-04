//! Export errors.

use std::io;
use thiserror::Error;

/// Export error.
#[derive(Error, Debug)]
pub enum ExportError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),

    #[error("Invalid options: {0}")]
    InvalidOptions(String),

    #[error("Encoding error: {0}")]
    Encoding(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("Resampling error: {0}")]
    Resample(String),

    #[error("Invalid audio data: {0}")]
    InvalidData(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, ExportError>;

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
