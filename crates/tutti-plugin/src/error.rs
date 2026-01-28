//! Error types for plugin bridge

use thiserror::Error;

#[derive(Error, Debug)]
pub enum BridgeError {
    #[error("Bridge connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Plugin load failed: {0}")]
    LoadFailed(String),

    #[error("IPC error: {0}")]
    IpcError(String),

    #[error("Shared memory error: {0}")]
    SharedMemoryError(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Bridge process crashed")]
    ProcessCrashed,

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("IO error")]
    Io(#[from] std::io::Error),

    #[error("Serialization error")]
    Serialization(#[from] bincode::Error),
}

pub type Result<T> = std::result::Result<T, BridgeError>;
