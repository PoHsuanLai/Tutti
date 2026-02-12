//! Multi-process plugin hosting client.
//!
//! Load VST2, VST3, and CLAP plugins in isolated server processes.
//! Server implementation is in `tutti-plugin-server`.
//!
//! # Example
//!
//! ```ignore
//! use tutti_plugin::{PluginClient, BridgeConfig};
//!
//! let (client, handle) = PluginClient::load(
//!     BridgeConfig::default(),
//!     "/path/to/plugin.vst3".into(),
//!     44100.0,
//! ).await?;
//! ```

pub mod error;
pub use error::{BridgeError, LoadStage, Result};

mod client;
pub use client::{PluginClient, PluginClientHandle};

mod handle;
pub use handle::PluginHandle;

pub mod bridge;
pub use bridge::PluginBridge;

pub mod instance;
pub use instance::{PluginInstance, ProcessContext, ProcessOutput};

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

pub mod inprocess_bridge;
pub use inprocess_bridge::{InProcessBridge, InProcessThreadHandle};

mod lockfree_bridge;
mod transport;
