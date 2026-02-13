use thiserror::Error;

#[derive(Debug, Clone, Error)]
pub enum Error {
    #[error("Invalid channel count: {0}")]
    InvalidChannelCount(String),

    #[error("Invalid speaker configuration: {0}")]
    InvalidSpeakerConfig(String),

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    #[cfg(feature = "spatial")]
    #[error("VBAP error: {0}")]
    VBAPError(String),
}

#[cfg(feature = "spatial")]
impl From<vbap::VBAPError> for Error {
    fn from(err: vbap::VBAPError) -> Self {
        Error::VBAPError(err.to_string())
    }
}

pub type Result<T> = core::result::Result<T, Error>;
