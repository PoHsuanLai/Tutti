//! Automation lanes and envelopes for Tutti.
//!
//! Provides [`AutomationLane`] nodes that read transport position
//! and output control signals based on envelope curves.
//!
//! # Example
//!
//! ```ignore
//! use tutti_automation::{AutomationLane, AutomationEnvelope, AutomationPoint, CurveType};
//!
//! // Create an envelope for filter cutoff
//! let mut envelope = AutomationEnvelope::new("filter_cutoff");
//! envelope.add_point(AutomationPoint::new(0.0, 0.2));  // Beat 0: 20%
//! envelope.add_point(AutomationPoint::with_curve(16.0, 1.0, CurveType::SCurve));  // Beat 16: 100%
//! envelope.add_point(AutomationPoint::new(32.0, 0.5));  // Beat 32: 50%
//!
//! // Create automation lane node
//! let lane = AutomationLane::new(envelope, transport_handle);
//!
//! // Add to graph - outputs control signal [0, 1]
//! let lane_id = engine.add_node(lane);
//! ```
//!
//! # Re-exports
//!
//! This crate re-exports the core types from `audio-automation`:
//! - [`AutomationEnvelope`] - Envelope with points and curves
//! - [`AutomationPoint`] - A single automation point
//! - [`CurveType`] - Interpolation curve types
//! - [`AutomationState`] - DAW-style automation states
//! - [`AutomationClip`] - Clip-based automation

mod lane;

pub use lane::{AutomationLane, LiveAutomationLane};

pub use audio_automation::{
    AutomationClip, AutomationEnvelope, AutomationPoint, AutomationState, CurveType,
};
