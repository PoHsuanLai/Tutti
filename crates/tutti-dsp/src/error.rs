//! Error types for tutti-dsp

use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    InvalidChannelCount(String),
    InvalidSpeakerConfig(String),
    InvalidParameter(String),
    #[cfg(feature = "spatial-audio")]
    VBAPError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidChannelCount(msg) => write!(f, "Invalid channel count: {}", msg),
            Error::InvalidSpeakerConfig(msg) => write!(f, "Invalid speaker configuration: {}", msg),
            Error::InvalidParameter(msg) => write!(f, "Invalid parameter: {}", msg),
            #[cfg(feature = "spatial-audio")]
            Error::VBAPError(msg) => write!(f, "VBAP error: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

// Convert VBAP errors to our error type (only when spatial-audio feature is enabled)
#[cfg(feature = "spatial-audio")]
impl From<vbap::VBAPError> for Error {
    fn from(err: vbap::VBAPError) -> Self {
        Error::VBAPError(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
