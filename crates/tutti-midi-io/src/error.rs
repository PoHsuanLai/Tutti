//! Error types for tutti-midi

use thiserror::Error;

/// Error type for tutti-midi operations
#[derive(Error, Debug)]
pub enum Error {
    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// MIDI file parse error
    #[error("MIDI parse error: {0}")]
    MidiFileParse(String),

    /// Unsupported MIDI timing format
    #[error("Unsupported MIDI timing format")]
    MidiUnsupportedTiming,

    /// MIDI port error
    #[error("MIDI port error: {0}")]
    MidiPort(String),

    /// MIDI device error
    #[error("MIDI device error: {0}")]
    MidiDevice(String),

    /// Invalid configuration
    #[error("Invalid config: {0}")]
    InvalidConfig(String),
}

impl From<midly::Error> for Error {
    fn from(e: midly::Error) -> Self {
        Error::MidiFileParse(e.to_string())
    }
}

impl From<midir::InitError> for Error {
    fn from(e: midir::InitError) -> Self {
        Error::MidiDevice(e.to_string())
    }
}

impl From<midir::ConnectError<midir::MidiOutput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiOutput>) -> Self {
        Error::MidiPort(e.to_string())
    }
}

impl From<midir::ConnectError<midir::MidiInput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiInput>) -> Self {
        Error::MidiPort(e.to_string())
    }
}

/// Result type for tutti-midi operations
pub type Result<T> = std::result::Result<T, Error>;
