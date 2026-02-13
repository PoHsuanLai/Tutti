//! Physical audio hardware input management and routing.

pub(crate) mod manager;
mod node;

pub use manager::InputDeviceInfo;
pub use node::{AudioInput, AudioInputBackend};
