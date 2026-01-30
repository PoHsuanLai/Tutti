//! Error types for neural audio processing.

use thiserror::Error;

/// Result type for neural audio operations
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during GPU operations (internal)
#[derive(Debug, Error)]
pub(crate) enum GpuError {
    /// Failed to initialize GPU backend
    #[error("Failed to initialize GPU backend: {0}")]
    BackendInitFailed(String),

    /// No suitable GPU device found
    #[error("No GPU device available")]
    NoGpuAvailable,

    /// Resource not found
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),

    /// Model loading error
    #[error("Failed to load model: {0}")]
    ModelLoadError(String),
}

/// Errors that can occur during neural audio processing
#[derive(Debug, Error)]
pub enum Error {
    /// GPU-related error
    #[error("GPU error: {0}")]
    Gpu(String),

    /// Tutti-core error
    #[error("Audio system error: {0}")]
    TuttiCore(#[from] tutti_core::Error),

    /// Model loading error
    #[error("Failed to load model: {0}")]
    ModelLoad(String),

    /// Model not found
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Inference error
    #[error("Inference error: {0}")]
    Inference(String),

    /// Invalid configuration
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Inference thread disconnected during initialization
    #[error("Inference thread disconnected during init")]
    InferenceThreadInit,

    /// Failed to send a request to the inference thread
    #[error("Inference thread send failed")]
    InferenceThreadSend,

    /// Failed to receive a response from the inference thread
    #[error("Inference thread recv failed")]
    InferenceThreadRecv,

    /// Invalid file path
    #[error("Invalid path: {0}")]
    InvalidPath(String),
}

impl From<GpuError> for Error {
    fn from(e: GpuError) -> Self {
        Error::Gpu(e.to_string())
    }
}
