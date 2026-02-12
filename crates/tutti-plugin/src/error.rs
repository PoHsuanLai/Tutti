//! Error types for plugin bridge

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStage {
    Scanning,
    Opening,
    Factory,
    Instantiation,
    Initialization,
    Setup,
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
    #[error("Bridge connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Plugin load failed at {stage} stage: {path}\n  Reason: {reason}")]
    LoadFailed {
        path: PathBuf,
        stage: LoadStage,
        reason: String,
    },

    #[error("Plugin error at {stage}: code {code:#x}")]
    PluginError { stage: LoadStage, code: i32 },

    #[error("IPC error: {0}")]
    IpcError(String),

    #[error("Shared memory error: {0}")]
    SharedMemoryError(String),

    #[error("Timeout after {duration_ms}ms: {operation}")]
    Timeout { operation: String, duration_ms: u64 },

    #[error("Bridge process crashed")]
    ProcessCrashed,

    #[error("Failed to save plugin state: {0}")]
    StateSaveError(String),

    #[error("Failed to restore plugin state: {0}")]
    StateRestoreError(String),

    #[error("Plugin editor error: {0}")]
    EditorError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] bincode::Error),
}

pub type Result<T> = std::result::Result<T, BridgeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_stage_display() {
        assert_eq!(LoadStage::Scanning.to_string(), "scanning");
        assert_eq!(LoadStage::Opening.to_string(), "opening library");
        assert_eq!(LoadStage::Factory.to_string(), "getting factory");
        assert_eq!(LoadStage::Instantiation.to_string(), "creating instance");
        assert_eq!(LoadStage::Initialization.to_string(), "initializing processor");
        assert_eq!(LoadStage::Setup.to_string(), "setting up audio");
        assert_eq!(LoadStage::Activation.to_string(), "activating");
    }

    #[test]
    fn test_bridge_error_display() {
        let err = BridgeError::ConnectionFailed("timeout".to_string());
        assert!(err.to_string().contains("timeout"));

        let err = BridgeError::Timeout {
            operation: "load".to_string(),
            duration_ms: 5000,
        };
        assert!(err.to_string().contains("5000ms"));
        assert!(err.to_string().contains("load"));

        let err = BridgeError::ProcessCrashed;
        assert_eq!(err.to_string(), "Bridge process crashed");
    }

    #[test]
    fn test_state_and_editor_errors() {
        let err = BridgeError::StateSaveError("failed to serialize".into());
        assert!(err.to_string().contains("save"));
        assert!(err.to_string().contains("failed to serialize"));

        let err = BridgeError::StateRestoreError("corrupt data".into());
        assert!(err.to_string().contains("restore"));
        assert!(err.to_string().contains("corrupt data"));

        let err = BridgeError::EditorError("no window handle".into());
        assert!(err.to_string().contains("editor"));
        assert!(err.to_string().contains("no window handle"));
    }
}
