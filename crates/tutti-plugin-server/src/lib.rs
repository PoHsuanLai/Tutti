//! Plugin server for isolated process hosting.
//!
//! Loads VST2, VST3, and CLAP plugins. Used by the `plugin-server` binary.
//! DAW applications should use `tutti-plugin` instead.

pub mod instance;
pub mod server;
pub mod transport;

#[cfg(feature = "vst2")]
pub mod vst2_loader;

#[cfg(feature = "vst3")]
pub mod vst3_loader;

#[cfg(feature = "clap")]
pub mod clap_loader;

pub use instance::{PluginInstance, ProcessContext, ProcessOutput};
pub use server::PluginServer;
pub use transport::{MessageTransport, TransportListener};

// Shared test utilities for plugin integration tests.
// VST2's `vst` crate uses a global `LOAD_POINTER` static during plugin loading
// that is not thread-safe. All plugin loading tests must be serialized.
#[cfg(test)]
pub(crate) mod test_utils {
    use std::sync::Mutex;
    pub static PLUGIN_LOAD_LOCK: Mutex<()> = Mutex::new(());
}

pub use tutti_plugin::{
    AudioIO, BridgeConfig, BridgeError, LoadStage, MidiEvent, MidiEventVec, NoteExpressionChanges,
    NoteExpressionType, NoteExpressionValue, ParameterChanges, ParameterFlags, ParameterInfo,
    ParameterPoint, ParameterQueue, PluginMetadata, Result, SampleFormat, TransportInfo,
};
