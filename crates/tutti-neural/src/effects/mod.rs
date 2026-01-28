//! Neural audio effects
//!
//! Builder infrastructure for neural models that process audio directly
//! (amp sims, compressors, reverbs, etc.)
//!
//! ## Architecture
//!
//! Mirrors the synthesis module pattern:
//! - `EffectBuilder` (internal) — loads model, creates `NeuralEffectNode` instances
//! - `SyncEffectBuilder` (public) — Send+Sync wrapper with dedicated inference thread
//!
//! Users bring their own trained models. This module provides the plumbing
//! to load them, wire up audio channels, and integrate into the audio graph.

pub(crate) mod effect_builder;
pub mod sync_builder;

pub use sync_builder::SyncEffectBuilder;
