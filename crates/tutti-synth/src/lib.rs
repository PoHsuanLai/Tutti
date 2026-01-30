//! Software synthesizers for Tutti
//!
//! This crate provides various software synthesizers that implement `AudioUnit`
//! and `MidiAudioUnit` for seamless integration with Tutti's audio graph and MIDI routing.
//!
//! ## Available Synths
//!
//! - **SoundFont** - SF2 sample-based synthesis via RustySynth
//!   - Multi-timbral (16 channels)
//!   - MIDI 1.0 compatible
//!   - Feature: `soundfont`
//!
//! ## Planned Synths
//!
//! - **PolySynth** - Simple polyphonic synthesizer
//!   - Multiple oscillator types (sine, saw, square, triangle)
//!   - ADSR envelopes
//!   - Basic filters
//!
//! - **WavetableSynth** - Wavetable synthesis
//!   - User-loadable wavetables
//!   - Morphing between tables
//!   - MPE support
//!
//! - **FMSynth** - FM synthesis (Yamaha DX7-style)
//!   - Multiple operator configurations
//!   - Algorithm matrix
//!
//! ## Usage
//!
//! ```ignore
//! use tutti::prelude::*;
//!
//! let engine = TuttiEngine::builder()
//!     .sample_rate(44100.0)
//!     .build()?;
//!
//! // Create a SoundFont synth
//! let synth = SoundFontUnit::new(soundfont, &settings);
//!
//! // Add to audio graph
//! let synth_node = engine.graph(|net| {
//!     net.add(Box::new(synth))
//! });
//!
//! // Route MIDI to synth (MIDI events will be processed automatically)
//! engine.transport().play();
//! ```

pub mod error;
pub use error::{Error, Result};

#[cfg(feature = "soundfont")]
pub mod soundfont;

#[cfg(feature = "soundfont")]
pub use soundfont::{SoundFontManager, SoundFontSynth, SoundFontUnit};

pub mod polysynth;
pub use polysynth::{Envelope, PolySynth, Waveform};
