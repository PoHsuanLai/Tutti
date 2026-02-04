//! SoundFont synthesis using RustySynth
//!
//! This module provides SoundFont (.sf2) synthesis for pattern clip rendering.
//! RustySynth is a high-quality SoundFont synthesizer that supports SF2 format.
//!
//! Note: This entire module is feature-gated by `soundfont` in lib.rs.

mod manager;
mod synthesizer;
mod unit;

pub use manager::{SoundFontHandle, SoundFontSystem};
pub use synthesizer::SoundFontSynth;
pub use unit::SoundFontUnit;
