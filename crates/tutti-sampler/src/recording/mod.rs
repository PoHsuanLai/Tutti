//! Recording session management and automation system.
//!
//! This module is internal to tutti-sampler. Access recording functionality
//! through [`SamplerSystem::recording()`] and [`SamplerSystem::automation()`].

pub(crate) mod config;
pub(crate) mod events;
pub(crate) mod manager;
pub(crate) mod session;

mod automation_lane;
pub(crate) mod automation_manager;
mod automation_target;

pub(crate) use config::{RecordingConfig, RecordingMode, RecordingSource};
pub(crate) use events::RecordingBuffer;
pub(crate) use session::{PunchEvent, RecordedData, RecordingSession, RecordingState, XRunEvent};

pub(crate) use automation_lane::{AutomationLane, AutomationRecordingConfig};
pub(crate) use automation_target::AutomationTarget;
