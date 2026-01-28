//! Error types for tutti-dsp

use std::fmt;

#[derive(Debug, Clone)]
pub enum Error {
    InvalidChannelCount(String),
    InvalidSpeakerConfig(String),
    InvalidParameter(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidChannelCount(msg) => write!(f, "Invalid channel count: {}", msg),
            Error::InvalidSpeakerConfig(msg) => write!(f, "Invalid speaker configuration: {}", msg),
            Error::InvalidParameter(msg) => write!(f, "Invalid parameter: {}", msg),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
