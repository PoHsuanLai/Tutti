//! SoundFont (.sf2) synthesis via RustySynth.

mod manager;
mod unit;

pub use manager::{SoundFontHandle, SoundFontSystem};
pub use rustysynth::{SoundFont, SynthesizerSettings};
pub use unit::SoundFontUnit;
