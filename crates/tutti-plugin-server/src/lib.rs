//! Plugin server for tutti-plugin
//!
//! This crate provides the server-side implementation for multi-process plugin hosting.
//! It loads and runs VST2, VST3, and CLAP plugins in an isolated process.
//!
//! This crate is used by the `plugin-server` binary and is not intended for direct use
//! by DAW applications. Use `tutti-plugin` for the client-side API.

pub mod instance;
pub mod server;
pub mod transport;

// Plugin loaders (feature-gated)
#[cfg(feature = "vst2")]
pub mod vst2_loader;

#[cfg(feature = "vst3")]
pub mod vst3_loader;

#[cfg(feature = "clap")]
pub mod clap_loader;

// Re-exports
pub use server::PluginServer;
pub use instance::{PluginInstance, ProcessContext, ProcessOutput};
pub use transport::{MessageTransport, TransportListener};

// Re-export shared types from tutti-plugin
pub use tutti_plugin::{
    BridgeConfig, BridgeError, LoadStage, Result,
    SampleFormat, PluginMetadata, AudioIO,
    MidiEvent, MidiEventVec,
    ParameterChanges, ParameterQueue, ParameterPoint,
    ParameterInfo, ParameterFlags,
    NoteExpressionChanges, NoteExpressionType, NoteExpressionValue,
    TransportInfo,
};
