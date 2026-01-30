//! SoundFont synthesis using RustySynth
//!
//! This module provides SoundFont (.sf2) synthesis for pattern clip rendering.
//! RustySynth is a high-quality SoundFont synthesizer that supports SF2 format.

#[cfg(feature = "soundfont")]
mod manager;

#[cfg(feature = "soundfont")]
mod synthesizer;

#[cfg(feature = "soundfont")]
mod unit;

#[cfg(feature = "soundfont")]
pub use manager::{SoundFontHandle, SoundFontManager};

#[cfg(feature = "soundfont")]
pub use synthesizer::SoundFontSynth;

#[cfg(feature = "soundfont")]
pub use unit::SoundFontUnit;

// Stub types when soundfont feature is disabled
#[cfg(not(feature = "soundfont"))]
pub struct SoundFontManager;

#[cfg(not(feature = "soundfont"))]
impl SoundFontManager {
    pub fn new(_sample_rate: u32) -> Self {
        Self
    }
}
