//! Multi-process plugin hosting for Tutti
//!
//! This crate provides the client-side API for loading VST2, VST3, and CLAP plugins
//! in isolated server processes. The server-side implementation is in `tutti-plugin-server`.
//!
//! ## Benefits
//!
//! - **Crash isolation**: Plugin crashes don't crash the DAW
//! - **Security**: Malicious plugins sandboxed from main process
//! - **Memory safety**: Separate address spaces
//! - **Platform compliance**: Required by macOS App Store
//!
//! ## Usage
//!
//! ```ignore
//! use tutti_plugin::{PluginClient, BridgeConfig};
//!
//! // Load plugin (spawns isolated server process)
//! let (client, handle) = PluginClient::load(
//!     BridgeConfig::default(),
//!     "/path/to/plugin.vst3".into(),
//!     44100.0,
//! ).await?;
//!
//! // Client implements AudioUnit trait for processing
//! // Clone is cheap - just Arc clones
//! let clone = client.clone();
//! ```

pub mod error;
pub use error::{BridgeError, LoadStage, Result};

mod client;
pub use client::{PluginClient, PluginClientHandle};

mod metadata;
pub use metadata::{AudioIO, PluginMetadata};

#[doc(hidden)]
pub mod protocol;

pub use protocol::{
    BridgeConfig, MidiEventVec, NoteExpressionChanges, NoteExpressionType, NoteExpressionValue,
    ParameterChanges, ParameterFlags, ParameterInfo, ParameterPoint, ParameterQueue, SampleFormat,
    TransportInfo,
};

pub use tutti_midi_io::MidiEvent;

#[doc(hidden)]
pub mod shared_memory;

mod registry;
pub use registry::{register_all_system_plugins, register_plugin, register_plugin_directory};

mod lockfree_bridge;
mod transport;
