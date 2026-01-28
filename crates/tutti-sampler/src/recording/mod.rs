//! Recording session management and automation system.

pub mod config;
pub mod events;
pub mod manager;
pub mod session;

// Automation modules
mod automation_lane;
pub(crate) mod automation_manager;
mod automation_target;

// Re-export recording types (for internal use within tutti-sampler)
pub(crate) use config::{RecordingConfig, RecordingMode, RecordingSource};
pub(crate) use events::RecordingBuffer;
pub(crate) use session::{RecordedData, RecordingSession, RecordingState};

// Re-export automation types (for internal use)
pub(crate) use automation_lane::{AutomationLane, AutomationRecordingConfig};
pub(crate) use automation_target::AutomationTarget;
