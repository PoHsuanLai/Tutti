//! Error types for neural audio processing.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("GPU error: {0}")]
    Gpu(String),

    #[error("Audio system error: {0}")]
    TuttiCore(#[from] tutti_core::Error),

    #[error("Failed to load model: {0}")]
    ModelLoad(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("Inference error: {0}")]
    Inference(String),

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Inference thread disconnected during init")]
    InferenceThreadInit,

    #[error("Inference thread send failed")]
    InferenceThreadSend,

    #[error("Inference thread recv failed")]
    InferenceThreadRecv,

    #[error("Invalid path: {0}")]
    InvalidPath(String),
}
