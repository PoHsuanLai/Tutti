//! Physical audio hardware input management and routing.

pub mod manager;
pub mod node;

pub use node::{AudioInput, AudioInputBackend};
