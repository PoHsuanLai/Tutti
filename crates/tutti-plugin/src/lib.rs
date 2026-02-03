//! Multi-process plugin hosting for Tutti
//!
//! This crate provides a multi-process architecture for loading VST2, VST3, and CLAP plugins
//! in isolated server processes. This is the **industry standard** approach used by professional
//! DAWs like Bitwig Studio, Logic Pro, and Ableton Live.
//!
//! ## Benefits
//!
//! - **Crash isolation**: Plugin crashes don't crash the DAW
//! - **Security**: Malicious plugins sandboxed from main process
//! - **Memory safety**: Separate address spaces
//! - **Platform compliance**: Required by macOS App Store
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────┐
//! │  Tutti Main Process                  │
//! │  ┌────────────────────────────────┐  │
//! │  │  PluginClient                  │  │
//! │  │  - Implements Plugin traits    │  │
//! │  │  - Cheap to clone (Arc)        │  │
//! │  │  - Sends audio via shm         │  │
//! │  │  - Receives via IPC            │  │
//! │  └──────────┬─────────────────────┘  │
//! └─────────────┼────────────────────────┘
//!               │ Unix socket / Named pipe
//!   ┌───────────▼────────────┐
//!   │  Plugin Server Process │
//!   │  ┌──────────────────┐  │
//!   │  │ PluginServer     │  │
//!   │  │ - Loads plugin   │  │
//!   │  │ - Calls VST API  │  │
//!   │  │ - Hosts GUI      │  │
//!   │  └──────────────────┘  │
//!   └────────────────────────┘
//! ```
//!
//! ## Features
//!
//! ### Audio Processing
//! - **32-bit and 64-bit audio** - Full support for both sample formats
//! - **Zero-copy shared memory** - Audio buffers passed via mmap
//! - **RT-safe lock-free design** - No mutexes in audio thread
//!
//! ### MIDI Support
//! - **Sample-accurate MIDI** - Frame-precise event timing (VST2, VST3)
//! - **Bidirectional MIDI** - Input and output events
//! - **All MIDI message types** - Notes, CC, pitch bend, poly pressure
//!
//! ### Automation & Transport
//! - **Sample-accurate automation** - Parameter changes with frame offsets (VST3)
//! - **Bidirectional automation** - Host → plugin, plugin → host
//! - **Full transport context** - Tempo, time signature, play state (VST2, VST3)
//! - **Musical position** - PPQ, bar position, cycle range
//!
//! ### Plugin Formats
//! - **VST2** - Legacy format with TimeInfo support
//! - **VST3** - Full process context and automation
//! - **CLAP** (experimental) - Modern open-source format
//!
//! ## Usage
//!
//! ```ignore
//! use tutti_plugin::{PluginClient, BridgeConfig};
//!
//! // Start plugin server process
//! let mut client = PluginClient::new(BridgeConfig::default())?;
//! client.init().await?;
//!
//! // Load plugin (VST2, VST3, CLAP, etc.)
//! client.load_plugin("/path/to/plugin.vst3", 44100.0).await?;
//!
//! // Process with MIDI, automation, and transport
//! let mut buffer = AudioBuffer { /* ... */ };
//! let midi_events = vec![MidiEvent::note_on(0, 0, 60, 100)];
//! let param_changes = ParameterChanges::new();
//! let transport = TransportInfo { tempo: 120.0, playing: true, /* ... */ };
//!
//! client.process_with_automation(&mut buffer, &midi_events, &param_changes, &transport);
//!
//! // Clone is cheap - just Arc clones, no factory calls
//! let clone = client.clone();
//! ```

pub mod client;
pub mod error;
pub mod instance;
pub mod lockfree_bridge;
pub mod metadata;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod shared_memory;
pub mod transport;

// VST2 loader (optional)
#[cfg(feature = "vst2")]
pub mod vst2_loader;

// VST3 loader (optional)
#[cfg(feature = "vst3")]
pub mod vst3_loader;

// CLAP loader (optional) - thin wrapper around clap-host crate
#[cfg(feature = "clap")]
mod clap;

#[cfg(feature = "clap")]
pub use clap::ClapInstance;

// Backwards compatibility alias
#[cfg(feature = "clap")]
pub mod clap_loader {
    pub use super::clap::ClapInstance;
}

// Re-exports
pub use client::PluginClient;
pub use error::{BridgeError, Result};
pub use instance::{PluginInstance, ProcessContext, ProcessOutput};
pub use protocol::{BridgeConfig, BridgeMessage, HostMessage, SharedBuffer};
pub use server::PluginServer;
pub use shared_memory::SharedAudioBuffer;

// Registry functions
pub use registry::{register_all_system_plugins, register_plugin, register_plugin_directory};

#[cfg(feature = "vst2")]
pub use registry::register_system_vst2_plugins;

#[cfg(feature = "vst3")]
pub use registry::register_system_vst3_plugins;

#[cfg(feature = "clap")]
pub use registry::register_system_clap_plugins;
