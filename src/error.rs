//! Centralized error type for the tutti umbrella crate.
//!
//! Wraps all subsystem errors so `?` propagates naturally across crate boundaries.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] tutti_core::Error),

    #[cfg(feature = "midi")]
    #[error("MIDI: {0}")]
    Midi(#[from] tutti_midi_io::Error),

    #[cfg(feature = "synth")]
    #[error("Synth: {0}")]
    Synth(#[from] tutti_synth::Error),

    #[cfg(feature = "sampler")]
    #[error("Sampler: {0}")]
    Sampler(#[from] tutti_sampler::Error),

    #[error("DSP: {0}")]
    Dsp(#[from] tutti_dsp::Error),

    #[cfg(feature = "plugin")]
    #[error("Plugin: {0}")]
    Plugin(#[from] tutti_plugin::BridgeError),

    #[cfg(feature = "neural")]
    #[error("Neural: {0}")]
    Neural(#[from] tutti_neural::Error),

    #[cfg(feature = "export")]
    #[error("Export: {0}")]
    Export(#[from] tutti_export::ExportError),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
