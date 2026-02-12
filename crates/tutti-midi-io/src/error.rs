//! Error types for the MIDI I/O subsystem.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("MIDI parse error: {0}")]
    MidiFileParse(String),

    #[error("Unsupported MIDI timing format")]
    MidiUnsupportedTiming,

    #[error("MIDI port error: {0}")]
    MidiPort(String),

    #[error("MIDI device error: {0}")]
    MidiDevice(String),

    #[error("Invalid config: {0}")]
    InvalidConfig(String),
}

impl From<midly::Error> for Error {
    fn from(e: midly::Error) -> Self {
        Error::MidiFileParse(e.to_string())
    }
}

#[cfg(feature = "midi-io")]
impl From<midir::InitError> for Error {
    fn from(e: midir::InitError) -> Self {
        Error::MidiDevice(e.to_string())
    }
}

#[cfg(feature = "midi-io")]
impl From<midir::ConnectError<midir::MidiOutput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiOutput>) -> Self {
        Error::MidiPort(e.to_string())
    }
}

#[cfg(feature = "midi-io")]
impl From<midir::ConnectError<midir::MidiInput>> for Error {
    fn from(e: midir::ConnectError<midir::MidiInput>) -> Self {
        Error::MidiPort(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
