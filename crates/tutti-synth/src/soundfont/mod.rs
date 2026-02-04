//! SoundFont (.sf2) synthesis via RustySynth.
//!
//! Provides [`SoundFontSynth`] for MIDI-driven sample playback and
//! [`SoundFontSystem`] for managing loaded SoundFonts.

mod manager;
mod synthesizer;
mod unit;

pub use manager::{SoundFontHandle, SoundFontSystem};
pub use synthesizer::SoundFontSynth;
pub use unit::SoundFontUnit;
