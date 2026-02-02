//! Error types for plugin bridge

use std::path::PathBuf;
use thiserror::Error;

/// Plugin loading stage for detailed error reporting
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    /// Scanning for plugin files
    Scanning,
    /// Opening plugin library
    Opening,
    /// Getting plugin factory
    Factory,
    /// Creating plugin instance
    Instantiation,
    /// Initializing plugin processor
    Initialization,
    /// Setting up audio processing
    Setup,
    /// Activating plugin
    Activation,
}

impl std::fmt::Display for LoadStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadStage::Scanning => write!(f, "scanning"),
            LoadStage::Opening => write!(f, "opening library"),
            LoadStage::Factory => write!(f, "getting factory"),
            LoadStage::Instantiation => write!(f, "creating instance"),
            LoadStage::Initialization => write!(f, "initializing processor"),
            LoadStage::Setup => write!(f, "setting up audio"),
            LoadStage::Activation => write!(f, "activating"),
        }
    }
}

#[derive(Error, Debug)]
pub enum BridgeError {
    /// Bridge connection failed
    #[error("Bridge connection failed: {0}")]
    ConnectionFailed(String),

    /// Plugin loading failed at specific stage
    #[error("Plugin load failed at {stage} stage: {path}\n  Reason: {reason}")]
    LoadFailed {
        path: PathBuf,
        stage: LoadStage,
        reason: String,
    },

    /// Plugin returned error code
    #[error("Plugin error at {stage}: code {code:#x}")]
    PluginError { stage: LoadStage, code: i32 },

    /// IPC communication error
    #[error("IPC error: {0}")]
    IpcError(String),

    /// Shared memory error
    #[error("Shared memory error: {0}")]
    SharedMemoryError(String),

    /// Operation timeout
    #[error("Timeout after {duration_ms}ms: {operation}")]
    Timeout {
        operation: String,
        duration_ms: u64,
    },

    /// Bridge process crashed
    #[error("Bridge process crashed")]
    ProcessCrashed,

    /// Protocol error
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
}

pub type Result<T> = std::result::Result<T, BridgeError>;
